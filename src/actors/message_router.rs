use anyhow::Result;
use arc_swap::ArcSwap;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, oneshot};

use crate::{
    actors::session::SessionActorHandle,
    config::CONFIG,
    entities::{agent::AgentKey, map::GameMap},
    game::events::BroadcastMessage,
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
        for _message in messages {
            todo!("implement message routing");
        }
    }
}
