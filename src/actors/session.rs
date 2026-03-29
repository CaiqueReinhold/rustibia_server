use std::sync::Arc;

use anyhow::Result;
use thiserror::Error;
use tokio::select;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::error;
use tracing::info;
use tracing::warn;

use super::{connection::ConnectionCommand, world::WorldCommand, ActorHandle};
use crate::actors::map_query::find_item_in_reach;
use crate::actors::map_query::get_map_desc_on_viewport;
use crate::actors::map_query::get_map_expansion;
use crate::actors::map_query::get_tile;
use crate::actors::map_query::retrieve_item;
use crate::actors::player_query::get_player_desc;
use crate::actors::BroadcastMessage;
use crate::config::CONFIG;
use crate::entities::items::ContainerId;
use crate::entities::items::ItemAttribute;
use crate::entities::items::ItemFlag;
use crate::entities::items::ItemGuid;
use crate::local_id::LocalIdMap;
use crate::messages::TextMessageType;
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
    #[error("Player is not spawned")]
    NotSpawned,
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
    containers: LocalIdMap<ItemGuid>,
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
                containers: LocalIdMap::new(),
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
        info!(
            session = self.session_id,
            "Session eceived command: {:?}", cmd
        );
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
        if handle.is_none() {
            let _ = self.connection.send(ConnectionCommand::Close).await;
            return Err(SessionError::FailedToInitialize.into());
        }

        self.player_key = handle;
        Ok(())
    }

    async fn send_position(&self, position: Position) -> Result<()> {
        self.connection
            .send(ConnectionCommand::SendPlayerMessage(
                ServerMessage::PlayerPosition { position },
            ))
            .await?;
        Ok(())
    }

    async fn pong(&self) -> Result<()> {
        self.connection
            .send(ConnectionCommand::SendPlayerMessage(ServerMessage::Pong))
            .await?;
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
            ClientMessage::UseItem {
                position,
                item_id,
                stack_index,
            } => self.handle_use_item(position, item_id, stack_index).await,
        }
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
        from: Position,
        item_id: ItemId,
        amount: u8,
        stack_index: u16,
        to: Position,
    ) -> Result<()> {
        let map = self.shared_map.load();

        let player_pos = map
            .agent_position(self.player_key.unwrap())
            .ok_or(SessionError::NotSpawned)?;

        if !player_pos.is_adjacent(&from) {
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::MoveItemDenied,
                ))
                .await?;
            return Ok(());
        }

        let item = retrieve_item(&map, &from, item_id, stack_index);
        let Some(item) = item else {
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::MoveItemDenied,
                ))
                .await?;
            return Ok(());
        };

        if item.amount < amount || item.config.has_flag(ItemFlag::Unmove) {
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::MoveItemDenied,
                ))
                .await?;
            return Ok(());
        }

        let denied = if to.is_container_coord() || to.is_inventory_coord() {
            // validate container is full
            // validate item fits in inventory slot
            // validate item requirement for slot
            !item.config.has_flag(ItemFlag::Take)
        } else {
            !map.can_drop_item(&to) && player_pos.in_viewport(&to)
            // check if target position can be reached (unsight flag)
        };

        if denied {
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::MoveItemDenied,
                ))
                .await?;
            return Ok(());
        }

        self.world
            .send(WorldCommand::MoveItem {
                agent: self.player_key.unwrap(),
                from,
                item_id,
                amount,
                stack_index,
                to,
            })
            .await?;

        Ok(())
    }

    async fn handle_use_item(
        &self,
        position: Position,
        item_id: ItemId,
        stack_index: u16,
    ) -> Result<()> {
        let map = self.shared_map.load();

        let player_pos = map
            .agent_position(self.player_key.unwrap())
            .ok_or(SessionError::NotSpawned)?;

        let send_error_ack = async || {
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::TextMessage {
                        text: "Item cannot be used".to_string(),
                        message_type: TextMessageType::ActionDenied,
                    },
                ))
                .await
                .and(
                    self.connection
                        .send(ConnectionCommand::SendPlayerMessage(
                            ServerMessage::UseItemAck,
                        ))
                        .await,
                )
        };

        if !player_pos.is_adjacent(&position) {
            info!("not adjacent");
            send_error_ack().await?;
            return Ok(());
        }

        let item = retrieve_item(&map, &position, item_id, stack_index);
        let Some(item) = item else {
            info!("cant retrieve item");
            send_error_ack().await?;
            return Ok(());
        };

        if !item.config.has_flag(ItemFlag::Usable) {
            info!("item not usable");
            send_error_ack().await?;
            return Ok(());
        }

        self.world
            .send(WorldCommand::UseItem {
                agent: self.player_key.unwrap(),
                guid: item.guid.clone(),
                position,
            })
            .await?;

        Ok(())
    }

    async fn route_broadcast(&mut self, msg: BroadcastMessage) -> Result<()> {
        info!(
            session = self.session_id,
            "Session received broadcast: {:?}", msg
        );
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
            BroadcastMessage::MoveAck { agent_key } => self.move_item_result(agent_key, true).await,
            BroadcastMessage::MoveDenied { agent_key } => {
                self.move_item_result(agent_key, false).await
            }
            BroadcastMessage::TileChanged { position } => self.tile_changed(position).await,
            BroadcastMessage::UseItemAck { agent_key, success } => {
                self.use_item_ack(agent_key, success).await
            }
            BroadcastMessage::OpenContainer {
                agent_key,
                guid,
                position,
            } => self.open_container(agent_key, guid, position).await,
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

    fn drop_unreachble_containers(&mut self) {
        let map = self.shared_map.load();
        let mut remove: Vec<ContainerId> = Vec::new();
        for guid in self.containers.iter_global() {
            if find_item_in_reach(&map, guid, self.player_key.unwrap()).is_none() {
                remove.push(self.containers.get_local(guid).unwrap());
            }
        }
        for id in remove {
            self.containers.remove_by_local(id);
        }
    }

    async fn agent_moved(
        &mut self,
        agent_key: AgentKey,
        direction: Direction,
        from_pos: Position,
    ) -> Result<()> {
        if self.player_key == Some(agent_key) {
            self.drop_unreachble_containers();
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

    async fn move_item_result(&self, agent_key: AgentKey, success: bool) -> Result<()> {
        if self.player_key == Some(agent_key) {
            if success {
                self.connection
                    .send(ConnectionCommand::SendPlayerMessage(
                        ServerMessage::MoveItemAck,
                    ))
                    .await?;
            } else {
                self.connection
                    .send(ConnectionCommand::SendPlayerMessage(
                        ServerMessage::MoveItemDenied,
                    ))
                    .await?;
            }
        }
        Ok(())
    }

    async fn tile_changed(&mut self, position: Position) -> Result<()> {
        self.drop_unreachble_containers();
        let map = self.shared_map.load();
        let player_pos = map
            .agent_position(self.player_key.unwrap())
            .ok_or(SessionError::NotSpawned)?;

        if player_pos.in_viewport(&position) {
            let tile = get_tile(&map, &position);
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::TileChanged {
                        position,
                        items: tile,
                    },
                ))
                .await?;
        }

        Ok(())
    }

    async fn use_item_ack(&self, agent_key: AgentKey, success: bool) -> Result<()> {
        if self.player_key == Some(agent_key) {
            if !success {
                self.connection
                    .send(ConnectionCommand::SendPlayerMessage(
                        ServerMessage::TextMessage {
                            text: "Cannot use this".to_string(),
                            message_type: TextMessageType::ActionDenied,
                        },
                    ))
                    .await?;
            }
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::UseItemAck,
                ))
                .await?;
        }
        Ok(())
    }

    async fn open_container(
        &mut self,
        agent_key: AgentKey,
        guid: ItemGuid,
        position: Position,
    ) -> Result<()> {
        if self.player_key != Some(agent_key) {
            return Ok(());
        }

        let map = self.shared_map.load();
        let item = if position.is_container_coord() {
            let parent_container = position.y as ContainerId;
            let Some(parent_guid) = self.containers.get_global(parent_container) else {
                return Ok(());
            };
            let Some(parent) = find_item_in_reach(&map, parent_guid, self.player_key.unwrap())
            else {
                return Ok(());
            };
            let Some(ref content) = parent.content else {
                warn!("container {parent_guid} has content vector none");
                return Ok(());
            };
            content.iter().find(|i| i.guid == guid)
        } else {
            find_item_in_reach(&map, &guid, self.player_key.unwrap())
        };

        let Some(item) = item else {
            return Ok(());
        };

        let Some(capacity) = item.config.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Capacity(c) => Some(c),
            _ => None,
        }) else {
            return Ok(());
        };
        let capacity = *capacity;
        let Some(ref content) = item.content else {
            return Ok(());
        };

        let title = item.get_name().to_owned();
        let items = content
            .iter()
            .map(|i| Some((i.item_id, i.amount)))
            .collect::<Vec<Option<(ItemId, u8)>>>()
            .into_boxed_slice();
        let container_id = self.containers.get_or_insert(guid.clone());

        self.connection
            .send(ConnectionCommand::SendPlayerMessage(
                ServerMessage::OpenContainer {
                    container_id,
                    capacity,
                    title,
                    items,
                },
            ))
            .await?;

        Ok(())
    }
}
