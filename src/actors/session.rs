use std::sync::Arc;

use anyhow::Ok;
use anyhow::Result;
use thiserror::Error;
use tokio::select;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::error;
use tracing::info;

use super::{connection::ConnectionCommand, world::WorldCommand, ActorHandle};
use crate::actors::BroadcastMessage;
use crate::config::CONFIG;
use crate::entities::{agent::AgentKey, items::ItemId, map::Position};
use crate::messages::{ClientMessage, Direction, ServerMessage};
use crate::persistence::player::PlayerRepository;

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("Session failed to initialize")]
    FailedToInitialize,
    #[error("Message type unknown or out of order")]
    WrongMessageType,
}

#[derive(Clone, Debug)]
pub enum SessionCommand {
    Close,
    Login {
        character_id: u32,
        auth_token: String,
    },
    PlayerSpawnResult(Option<AgentKey>),
    PlayerPosition(Position),
    ReceivePlayerMessage(ClientMessage),
}

pub struct SessionActor {
    session_id: String,
    rx: mpsc::Receiver<SessionCommand>,
    brx: broadcast::Receiver<BroadcastMessage>,
    self_handle: ActorHandle<SessionCommand>,
    connection: ActorHandle<ConnectionCommand>,
    world: ActorHandle<WorldCommand>,
    player_handle: Option<AgentKey>,
    player_repo: Arc<PlayerRepository>,
}

impl SessionActor {
    pub async fn start(
        session_id: String,
        conn_rx: oneshot::Receiver<ActorHandle<ConnectionCommand>>,
        world: ActorHandle<WorldCommand>,
        player_repo: Arc<PlayerRepository>,
        receiver: broadcast::Receiver<BroadcastMessage>,
    ) -> Result<ActorHandle<SessionCommand>> {
        let connection = conn_rx.await?;
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);

        let self_handle = ActorHandle { tx };

        let actor = Self {
            session_id,
            rx,
            self_handle: self_handle.clone(),
            connection,
            world,
            player_handle: None,
            player_repo,
            brx: receiver,
        };

        tokio::spawn(actor.run());

        Ok(self_handle)
    }

    async fn run(mut self) {
        info!(session = self.session_id, "Session actor started");
        loop {
            let result = select! {
                cmd = self.rx.recv() => self.route_command(cmd.unwrap()).await,
                msg = self.brx.recv() => self.route_broadcast(msg.unwrap()).await
            };
            if let Err(e) = result {
                error!("Error on session command: {e}");
                break;
            }
        }
    }

    async fn route_command(&mut self, cmd: SessionCommand) -> Result<()> {
        match cmd {
            SessionCommand::Login {
                character_id,
                auth_token,
            } => self.login(character_id, auth_token).await,
            SessionCommand::Close => self.close_connection().await,
            SessionCommand::ReceivePlayerMessage(msg) => self.handle_client_message(msg).await,
            SessionCommand::PlayerSpawnResult(handle) => self.spawn_result(handle).await,
            SessionCommand::PlayerPosition(pos) => self.send_position(pos).await,
        }
    }

    async fn route_broadcast(&self, msg: BroadcastMessage) -> Result<()> {
        match msg {
            BroadcastMessage::PlayerMoved {
                agent_key,
                direction,
            } => Ok(()),
            BroadcastMessage::PlayerSpawned {
                agent_key,
                position,
            } => Ok(()),
        }
    }

    async fn close_connection(&self) -> Result<()> {
        self.connection.send(ConnectionCommand::Close).await?;
        Ok(())
    }

    async fn login(&self, character_id: u32, _auth_token: String) -> Result<()> {
        self.connection.send(ConnectionCommand::AuthOk).await?; // TODO

        let player = self.player_repo.get_by_id(character_id).await?;
        self.world
            .send(WorldCommand::SpawnPlayer {
                player,
                session: self.self_handle.clone(),
            })
            .await
            .unwrap();
        Ok(())
    }

    async fn spawn_result(&mut self, handle: Option<AgentKey>) -> Result<()> {
        if handle.is_none() {
            let _ = self.connection.send(ConnectionCommand::Close).await;
            return Err(SessionError::FailedToInitialize.into());
        }

        self.player_handle = handle;
        Ok(())
    }

    async fn handle_client_message(&self, command: ClientMessage) -> Result<()> {
        match command {
            ClientMessage::Login { .. } => Err(SessionError::WrongMessageType.into()),
            ClientMessage::MovePlayer { direction } => self.handle_move_player(direction).await,
            ClientMessage::MoveItem {
                from,
                item_id,
                amount,
                stack_index,
                to,
            } => {
                self.handle_move_item(from, item_id, amount, stack_index, to)
                    .await
            }
        }
    }

    async fn handle_move_player(&self, direction: Direction) -> Result<()> {
        if self.player_handle.is_none() {
            return Err(SessionError::WrongMessageType.into());
        }

        let _ = self
            .world
            .send(WorldCommand::Walk {
                direction,
                actor: self.player_handle.unwrap(),
                session: self.self_handle.clone(),
            })
            .await;
        Ok(())
    }

    async fn handle_move_item(
        &self,
        _from: Position,
        _item_id: ItemId,
        _amount: u8,
        _stack_index: u16,
        _to: Position,
    ) -> Result<()> {
        todo!()
    }

    async fn send_position(&self, position: Position) -> Result<()> {
        self.connection
            .send(ConnectionCommand::SendPlayerMessage(
                ServerMessage::PlayerPosition { position },
            ))
            .await?;
        Ok(())
    }
}
