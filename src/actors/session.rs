use std::sync::Arc;

use anyhow::Result;
use thiserror::Error;
use tokio::select;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::error;
use tracing::info;

use super::{connection::ConnectionCommand, world::WorldCommand, ActorHandle};
use crate::actors::map_query::get_map_desc_on_viewport;
use crate::actors::map_query::get_map_expansion;
use crate::actors::player_query::get_player_desc;
use crate::actors::BroadcastMessage;
use crate::config::CONFIG;
use arc_swap::ArcSwap;

use crate::entities::{
    agent::AgentKey,
    items::ItemId,
    map::GameMap,
    position::{Direction, Position},
};
use crate::messages::{ClientMessage, ServerMessage};
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
    player_key: Option<AgentKey>,
    player_repo: Arc<PlayerRepository>,
    shared_map: Arc<ArcSwap<GameMap>>,
}

impl SessionActor {
    pub fn start(
        session_id: String,
        conn_rx: oneshot::Receiver<ActorHandle<ConnectionCommand>>,
        world: ActorHandle<WorldCommand>,
        player_repo: Arc<PlayerRepository>,
        receiver: broadcast::Receiver<BroadcastMessage>,
        shared_map: Arc<ArcSwap<GameMap>>,
    ) -> ActorHandle<SessionCommand> {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let self_handle = ActorHandle { tx };

        let self_handle_clone = self_handle.clone();
        tokio::spawn(async move {
            let connection = match conn_rx.await {
                Ok(c) => c,
                Err(_) => return,
            };
            let actor = Self {
                session_id,
                rx,
                self_handle: self_handle_clone,
                connection,
                world,
                player_key: None,
                player_repo,
                brx: receiver,
                shared_map,
            };
            actor.run().await;
        });

        self_handle
    }

    async fn run(mut self) {
        info!(session = self.session_id, "Session actor started");
        loop {
            let result = select! { biased;
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
        info!("Session received spawn result: {:?}", handle);
        if handle.is_none() {
            let _ = self.connection.send(ConnectionCommand::Close).await;
            return Err(SessionError::FailedToInitialize.into());
        }

        self.player_key = handle;
        Ok(())
    }

    async fn handle_client_message(&self, command: ClientMessage) -> Result<()> {
        if self.player_key.is_none() {
            return Err(SessionError::WrongMessageType.into());
        }
        match command {
            ClientMessage::Ping => self.pong().await,
            ClientMessage::Login { .. } => Err(SessionError::WrongMessageType.into()),
            ClientMessage::MovePlayer { direction } => self.handle_move_player(direction).await,
            ClientMessage::GetPlayerPosition => self.handle_get_position().await,
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

    async fn pong(&self) -> Result<()> {
        self.connection
            .send(ConnectionCommand::SendPlayerMessage(ServerMessage::Pong))
            .await?;
        Ok(())
    }

    async fn handle_move_player(&self, direction: Direction) -> Result<()> {
        let map = self.shared_map.load();
        if let Some(position) = map.agent_position(self.player_key.unwrap()) {
            let target_position = position.clone() + direction.clone();
            if !map.can_move(&target_position, self.player_key.unwrap()) {
                return self.handle_get_position().await;
            }

            let _ = self
                .world
                .send(WorldCommand::Walk {
                    direction,
                    actor: self.player_key.unwrap(),
                    session: self.self_handle.clone(),
                })
                .await;
        } else {
            return Err(SessionError::WrongMessageType.into());
        }

        Ok(())
    }

    async fn handle_get_position(&self) -> Result<()> {
        let map = self.shared_map.load();
        if let Some(position) = map.agent_position(self.player_key.unwrap()) {
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::PlayerPosition {
                        position: position.clone(),
                    },
                ))
                .await?;
        } else {
            return Err(SessionError::WrongMessageType.into());
        }
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

    async fn route_broadcast(&self, msg: BroadcastMessage) -> Result<()> {
        match msg {
            BroadcastMessage::AgentMoved {
                agent_key,
                direction,
                from_pos,
            } => self.agent_moved(agent_key, direction, from_pos).await,
            BroadcastMessage::PlayerSpawned {
                agent_key,
                position,
            } => self.player_spawned(agent_key, position).await,
        }
    }

    async fn player_spawned(&self, agent_key: AgentKey, position: Position) -> Result<()> {
        if self.player_key == Some(agent_key) {
            let map = self.shared_map.load();

            let tiles = get_map_desc_on_viewport(&map, &position);
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::DescribeMap { tiles },
                ))
                .await?;

            let player_desc = get_player_desc(&map, self.player_key.unwrap());
            if let Some(pdesc_msg) = player_desc {
                self.connection
                    .send(ConnectionCommand::SendPlayerMessage(pdesc_msg))
                    .await?;
            } else {
                return Err(SessionError::FailedToInitialize.into());
            }
            Ok(())
        } else {
            // check if player is in viewport
            // send player data if it is
            Ok(())
        }
    }

    async fn agent_moved(
        &self,
        agent_key: AgentKey,
        direction: Direction,
        from_pos: Position,
    ) -> Result<()> {
        if self.player_key == Some(agent_key) {
            let map = self.shared_map.load();
            let tiles = get_map_expansion(&map, &from_pos, &direction);
            let to_pos = from_pos + direction;
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::PlayerWalkAck {
                        position: to_pos,
                        tiles,
                    },
                ))
                .await?;
            Ok(())
        } else {
            Ok(())
        }
    }
}
