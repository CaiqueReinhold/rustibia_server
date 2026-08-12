use anyhow::Result;
use arc_swap::ArcSwap;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::{
    actors::session::{SessionActorHandle, SessionCommand},
    config::CONFIG,
    entities::{
        agent::AgentKey,
        chat::ChannelId,
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
    },
    Unsubscribe {
        agent_key: AgentKey,
    },
    Broadcast {
        messages: Vec<BroadcastMessage>,
    },
    DeliverPrivateMessage {
        author: AgentKey,
        recipient: AgentKey,
        message: String,
    },
    DeliverChannelMessage {
        author: AgentKey,
        recipients: Vec<AgentKey>,
        channel_id: ChannelId,
        message: String,
        tx: oneshot::Sender<Vec<AgentKey>>,
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
        self.handle.unsubscribe(self.agent_key);
    }
}

impl MessageRouterActorHandle {
    pub fn subscribe(
        &self,
        agent_key: AgentKey,
        session: SessionActorHandle,
    ) -> Result<MessageRouterGuard> {
        if self
            .tx
            .try_send(MessageRouterCommand::Subscribe { agent_key, session })
            .is_ok()
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

    pub fn unsubscribe(&self, agent_key: AgentKey) {
        let _ = self
            .tx
            .try_send(MessageRouterCommand::Unsubscribe { agent_key });
    }

    pub async fn broadcast(&self, messages: Vec<BroadcastMessage>) {
        let _ = self
            .tx
            .send(MessageRouterCommand::Broadcast { messages })
            .await;
    }

    pub async fn deliver_private_message(
        &self,
        author: AgentKey,
        recipient: AgentKey,
        message: String,
    ) {
        let _ = self
            .tx
            .send(MessageRouterCommand::DeliverPrivateMessage {
                author,
                recipient,
                message,
            })
            .await;
    }

    /// Delivers one channel message to every recipient and returns the keys it could not
    /// reach, so the caller can prune them. Batched on purpose: this actor is also the
    /// fan-out path for every world broadcast, so one round-trip per message keeps chat
    /// traffic off the critical path for movement and tile updates.
    pub async fn deliver_channel_message(
        &self,
        author: AgentKey,
        recipients: Vec<AgentKey>,
        channel_id: ChannelId,
        message: String,
    ) -> Vec<AgentKey> {
        let (tx, rx) = oneshot::channel();
        if self
            .tx
            .send(MessageRouterCommand::DeliverChannelMessage {
                author,
                recipients,
                channel_id,
                message,
                tx,
            })
            .await
            .is_err()
        {
            warn!("Router is gone; channel message dropped and no members pruned");
            return Vec::new();
        }
        match rx.await {
            Ok(dead) => dead,
            Err(_) => {
                warn!("Router dropped the reply for a channel message; no members pruned");
                Vec::new()
            }
        }
    }

    #[cfg(test)]
    pub fn for_test() -> (Self, mpsc::Receiver<MessageRouterCommand>) {
        let (tx, rx) = mpsc::channel(64);
        (Self { tx }, rx)
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
        info!("Message router actor started");
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
            MessageRouterCommand::Subscribe { agent_key, session } => {
                self.subscribe(agent_key, session)
            }
            MessageRouterCommand::Unsubscribe { agent_key } => self.unsubscribe(agent_key),
            MessageRouterCommand::Broadcast { messages } => self.broadcast(messages).await,
            MessageRouterCommand::DeliverPrivateMessage {
                author,
                recipient,
                message,
            } => self.deliver_private_message(author, recipient, message),
            MessageRouterCommand::DeliverChannelMessage {
                author,
                recipients,
                channel_id,
                message,
                tx,
            } => {
                let dead = self.deliver_channel_message(author, recipients, channel_id, message);
                let _ = tx.send(dead);
            }
        }
    }

    fn subscribe(&mut self, agent_key: AgentKey, session: SessionActorHandle) {
        if self.session_map.contains_key(&agent_key) {
            return;
        }

        self.session_map.insert(agent_key, session);
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
            BroadcastMessage::AgentChangedDirection { position, .. } => self.send_to_rect(
                message,
                map,
                Rect::player_viewport(position),
                position.z,
                None,
            ),
            BroadcastMessage::AgentMoved {
                agent_key,
                from_position,
                to_position,
                ..
            } => {
                let from_viewport = Rect::player_viewport(from_position);
                let to_viewport = Rect::player_viewport(to_position);
                self.send_to_rect(
                    message,
                    map,
                    Rect::new(
                        u16::min(from_viewport.min_x(), to_viewport.min_x()),
                        u16::min(from_viewport.min_y(), to_viewport.min_y()),
                        u16::max(from_viewport.max_x(), to_viewport.max_x()),
                        u16::max(from_viewport.max_y(), to_viewport.max_y()),
                    ),
                    to_position.z,
                    Some(*agent_key),
                );

                self.send_to(message, agent_key);
            }
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
            BroadcastMessage::MoveItemDenied { agent_key, .. } => {
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
                self.send_to_rect(
                    message,
                    map,
                    Rect::player_viewport(position),
                    position.z,
                    None,
                );
                self.send_to(message, agent_key); // player was already removed from the map, send using key.
            }
            BroadcastMessage::PlayerSpawned { position, .. } => {
                self.send_to_rect(
                    message,
                    map,
                    Rect::player_viewport(position),
                    position.z,
                    None,
                );
            }
            BroadcastMessage::AgentWalkDenied { agent_key } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::TileChanged { position } => {
                self.send_to_rect(
                    message,
                    map,
                    Rect::player_viewport(position),
                    position.z,
                    None,
                );
            }
            BroadcastMessage::UpdateContainer { item } => match &item.placement {
                ItemPlacement::Inventory(_slot, agent_key) => {
                    self.send_to(message, agent_key);
                }
                ItemPlacement::Map(pos) => {
                    self.send_to_rect(message, map, Rect::player_viewport(pos), pos.z, None);
                }
            },
            BroadcastMessage::UpdateInventorySlot { agent_key, .. } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::UpdatePlayerCapacity { agent_key } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::UseItemDenied { agent_key, .. } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::LogoutDenied { agent_key } => {
                self.send_to(message, agent_key);
            }
            BroadcastMessage::AgentSaid { agent_key, message } => {
                todo!()
            }
        }
    }

    fn send_to_rect(
        &mut self,
        message: &BroadcastMessage,
        map: &GameMap,
        rect: Rect,
        z: u8,
        originator: Option<AgentKey>,
    ) {
        iter_visible_floors(z)
            .flat_map(|floor| map.iter_agents_in_rect(&rect, floor))
            .for_each(|agent_key| {
                if Some(*agent_key) != originator {
                    self.send_to(message, agent_key)
                }
            });
    }

    /// Deliver `message` once to every agent whose viewport intersects any of
    /// `regions`. A single rect (or the per-floor expansion of one) can never
    /// yield a duplicate — tiles and floors partition space — so dups only
    /// arise where two regions overlap; the `seen` set collapses those.
    fn send_to_rects(&mut self, message: &BroadcastMessage, map: &GameMap, regions: &[(Rect, u8)]) {
        let mut seen: HashSet<AgentKey> = HashSet::new();
        for (rect, z) in regions {
            for floor in iter_visible_floors(*z) {
                for agent_key in map.iter_agents_in_rect(rect, floor) {
                    if seen.insert(*agent_key) {
                        self.send_to(message, agent_key);
                    }
                }
            }
        }
    }

    /// The single place a failed send to a session is interpreted. Returns whether the
    /// message was delivered, so callers that track membership can prune.
    fn handle_send_result(
        &mut self,
        agent_key: AgentKey,
        session: &SessionActorHandle,
        result: Result<(), TrySendError<SessionCommand>>,
    ) -> bool {
        match result {
            Ok(()) => true,
            Err(TrySendError::Closed(..)) => {
                self.unsubscribe(agent_key);
                false
            }
            Err(TrySendError::Full(..)) => {
                session.close();
                self.unsubscribe(agent_key);
                false
            }
        }
    }

    fn send_to(&mut self, message: &BroadcastMessage, agent_key: &AgentKey) {
        let Some(session) = self.session_map.get(agent_key).cloned() else {
            return;
        };
        let result = session.receive_broadcast(message.clone());
        self.handle_send_result(*agent_key, &session, result);
    }

    fn deliver_private_message(&mut self, author: AgentKey, recipient: AgentKey, message: String) {
        let Some(session) = self.session_map.get(&recipient).cloned() else {
            return;
        };
        let result = session.receive_chat_private(author, message);
        self.handle_send_result(recipient, &session, result);
    }

    /// Returns the recipients that could not be reached.
    fn deliver_channel_message(
        &mut self,
        author: AgentKey,
        recipients: Vec<AgentKey>,
        channel_id: ChannelId,
        message: String,
    ) -> Vec<AgentKey> {
        let mut dead = Vec::new();
        for recipient in recipients {
            let Some(session) = self.session_map.get(&recipient).cloned() else {
                dead.push(recipient);
                continue;
            };
            let result = session.receive_chat_channel(author, channel_id, message.clone());
            if !self.handle_send_result(recipient, &session, result) {
                dead.push(recipient);
            }
        }
        dead
    }
}
