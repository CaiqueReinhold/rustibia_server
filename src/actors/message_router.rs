use anyhow::Result;
use arc_swap::ArcSwap;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::{
    mpsc::{self, error::TrySendError},
    oneshot,
};

use crate::{
    actors::session::SessionActorHandle,
    config::CONFIG,
    entities::{
        agent::AgentKey,
        map::GameMap,
        position::{ItemPlacement, Rect},
    },
    game::{events::BroadcastMessage, map_query::iter_visible_floors},
};

#[derive(Debug)]
pub enum MessageRouterCommand {
    Subscribe {
        agent_key: AgentKey,
        session: SessionActorHandle,
        tx: oneshot::Sender<bool>,
    },
    Unsubscribe {
        agent_key: AgentKey,
    },
    Broadcast {
        messages: Vec<BroadcastMessage>,
    },
}

#[derive(Debug)]
pub struct MessageRouterGuard {
    agent_key: AgentKey,
    handle: MessageRouterActorHandle,
}

#[derive(Clone, Debug)]
pub struct MessageRouterActorHandle {
    tx: mpsc::Sender<MessageRouterCommand>,
}

#[derive(Debug)]
pub struct MessageRouterActor {
    rx: mpsc::Receiver<MessageRouterCommand>,
    shared_map: Arc<ArcSwap<GameMap>>,
    session_map: HashMap<AgentKey, SessionActorHandle>,
}

impl Drop for MessageRouterGuard {
    fn drop(&mut self) {
        let handle = self.handle.clone();
        let agent_key = self.agent_key;
        tokio::spawn(async move {
            let _ = handle.unsubscribe(agent_key).await;
        });
    }
}

impl MessageRouterActorHandle {
    pub async fn subscribe(
        &self,
        agent_key: AgentKey,
        session: SessionActorHandle,
    ) -> Result<MessageRouterGuard> {
        let (tx, rx) = oneshot::channel();
        if self
            .tx
            .send(MessageRouterCommand::Subscribe {
                agent_key,
                session,
                tx,
            })
            .await
            .is_ok()
            && let Ok(true) = rx.await
        {
            return Ok(MessageRouterGuard {
                agent_key,
                handle: MessageRouterActorHandle {
                    tx: self.tx.clone(),
                },
            });
        }

        Err(anyhow::anyhow!("Failed to subscribe"))
    }

    pub async fn unsubscribe(&self, agent_key: AgentKey) {
        let _ = self
            .tx
            .send(MessageRouterCommand::Unsubscribe { agent_key })
            .await;
    }

    pub async fn broadcast(&self, messages: Vec<BroadcastMessage>) {
        let _ = self
            .tx
            .send(MessageRouterCommand::Broadcast { messages })
            .await;
    }
}

impl MessageRouterActor {
    pub fn start(shared_map: Arc<ArcSwap<GameMap>>) -> MessageRouterActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);

        tokio::spawn(async move {
            let actor = Self {
                rx,
                shared_map: shared_map.clone(),
                session_map: HashMap::new(),
            };
            actor.run().await;
        });

        MessageRouterActorHandle { tx }
    }

    pub async fn run(mut self) {
        loop {
            let command = self.rx.recv().await;
            match command {
                Some(command) => self.handle_command(command).await,
                None => break,
            }
        }
    }

    async fn handle_command(&mut self, command: MessageRouterCommand) {
        match command {
            MessageRouterCommand::Subscribe {
                agent_key,
                session,
                tx,
            } => self.subscribe(agent_key, session, tx),
            MessageRouterCommand::Unsubscribe { agent_key } => self.unsubscribe(agent_key),
            MessageRouterCommand::Broadcast { messages } => self.broadcast(messages).await,
        }
    }

    fn subscribe(
        &mut self,
        agent_key: AgentKey,
        session: SessionActorHandle,
        tx: oneshot::Sender<bool>,
    ) {
        if self.session_map.contains_key(&agent_key) {
            let _ = tx.send(false);
            return;
        }

        self.session_map.insert(agent_key, session);

        if tx.send(true).is_err() {
            self.unsubscribe(agent_key);
        }
    }

    fn unsubscribe(&mut self, agent_key: AgentKey) {
        self.session_map.remove(&agent_key);
    }

    async fn broadcast(&mut self, messages: Vec<BroadcastMessage>) {
        let map = self.shared_map.load();
        for message in messages {
            self.route_to_recipients(&message, &map);
        }
    }

    fn route_to_recipients(&mut self, message: &BroadcastMessage, map: &GameMap) {
        match message {
            BroadcastMessage::AgentChangedDirection { position, .. } => {
                self.send_to_rect(message, map, Rect::player_viewport(position), position.z)
            }
            BroadcastMessage::AgentMoved {
                from_position,
                to_position,
                ..
            } => self.send_to_rect(
                message,
                map,
                Rect::new(
                    u16::min(from_position.x, to_position.x),
                    u16::min(from_position.y, to_position.y),
                    u16::max(from_position.x, to_position.x),
                    u16::max(from_position.y, to_position.y),
                ),
                to_position.z,
            ),
            BroadcastMessage::AgentTeleport {
                from_position,
                to_position,
                ..
            } => {
                // Origin and destination viewports can overlap, so dedup to
                // avoid delivering the teleport twice to agents in the overlap.
                self.send_to_rects(
                    message,
                    map,
                    &[
                        (Rect::player_viewport(from_position), from_position.z),
                        (Rect::player_viewport(to_position), to_position.z),
                    ],
                );
            }
            BroadcastMessage::MoveAck { agent_key } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::MoveDenied { agent_key, .. } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::OpenContainer { agent_key, .. } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::PlayerDespawned {
                agent_key,
                position,
                ..
            } => {
                self.send_to_rect(message, map, Rect::player_viewport(position), position.z);
                self.send_to(message, agent_key); // player was already removed from the map, send using key.
            }
            BroadcastMessage::PlayerSpawned { position, .. } => {
                self.send_to_rect(message, map, Rect::player_viewport(position), position.z);
            }
            BroadcastMessage::PlayerWalkDenied { agent_key } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::TileChanged { position } => {
                self.send_to_rect(message, map, Rect::player_viewport(position), position.z);
            }
            BroadcastMessage::UpdateContainer { item } => match &item.placement {
                ItemPlacement::Inventory(_slot, agent_key) => {
                    self.send_to(message, agent_key);
                }
                ItemPlacement::Map(pos) => {
                    self.send_to_rect(message, map, Rect::player_viewport(pos), pos.z);
                }
            },
            BroadcastMessage::UpdateInventorySlot { agent_key, .. } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::UpdatePlayerCapacity { agent_key } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::UseItemAck { agent_key, .. } => {
                self.send_to(message, agent_key);
            }
        }
    }

    fn send_to_rect(&mut self, message: &BroadcastMessage, map: &GameMap, rect: Rect, z: u8) {
        iter_visible_floors(z)
            .flat_map(|floor| map.get_agents_at_rect(&rect, floor))
            .for_each(|agent_key| self.send_to(message, agent_key));
    }

    /// Deliver `message` once to every agent whose viewport intersects any of
    /// `regions`. A single rect (or the per-floor expansion of one) can never
    /// yield a duplicate — tiles and floors partition space — so dups only
    /// arise where two regions overlap; the `seen` set collapses those.
    fn send_to_rects(&mut self, message: &BroadcastMessage, map: &GameMap, regions: &[(Rect, u8)]) {
        let mut seen: HashSet<AgentKey> = HashSet::new();
        for (rect, z) in regions {
            for floor in iter_visible_floors(*z) {
                for agent_key in map.get_agents_at_rect(rect, floor) {
                    if seen.insert(*agent_key) {
                        self.send_to(message, agent_key);
                    }
                }
            }
        }
    }

    fn send_to(&mut self, message: &BroadcastMessage, agent_key: &AgentKey) {
        if let Some(session) = self.session_map.get(agent_key)
            && let Err(error) = session.receive_broadcast(message.clone())
        {
            match error {
                TrySendError::Closed(..) => self.unsubscribe(*agent_key),
                TrySendError::Full(..) => {
                    session.close();
                    self.unsubscribe(*agent_key);
                }
            }
        }
    }
}
