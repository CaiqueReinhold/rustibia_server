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
use crate::actors::map_query::{
    find_item_in_reach, find_parent_container, get_map_desc_on_viewport, get_map_expansion,
    get_tile, retrieve_item,
};
use crate::actors::player_query::find_item_in_slot;
use crate::actors::player_query::get_player_desc;
use crate::actors::world::BroadcastMessage;
use crate::config::CONFIG;
use crate::entities::agent::Agent;
use crate::entities::agent::Facing;
use crate::entities::items::{ContainerId, ItemAttribute, ItemFlag, ItemGuid};
use crate::entities::player::InventorySlot;
use crate::entities::position::ItemPlacement;
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
    #[error("Invalid State")]
    InvalidState,
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
    agents: LocalIdMap<AgentKey>,
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
                agents: LocalIdMap::new(),
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
        let _ = self.connection.send(ConnectionCommand::Close).await;
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
                player: Agent::from_player(player),
                session: self.self_handle.clone(),
            })
            .await
            .unwrap();
        Ok(())
    }

    async fn spawn_result(&mut self, handle: Option<AgentKey>) -> Result<()> {
        if handle.is_none() {
            return Err(SessionError::FailedToInitialize.into());
        }

        self.player_key = handle;
        self.agents.get_or_insert(handle.unwrap());
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

    async fn handle_client_message(&mut self, command: ClientMessage) -> Result<()> {
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
            ClientMessage::CloseContainer { container_id } => {
                self.containers.remove_by_local(container_id);
                Ok(())
            }
            ClientMessage::OpenParentContainer { container_id } => {
                self.handle_open_parent_container(container_id).await
            }
            ClientMessage::ChangeDirection { direction } => {
                self.handle_change_direction(direction).await
            }
        }
    }

    async fn handle_move_player(&self, direction: Direction) -> Result<()> {
        let map = self.shared_map.load();
        if let Some(position) = map.agent_position(self.player_key.unwrap()) {
            let target_position = position.clone() + direction;
            if !map.can_move(&target_position, self.player_key.unwrap()) {
                return self.walk_denied(self.player_key.unwrap()).await;
            }

            let _ = self
                .world
                .send(WorldCommand::Walk {
                    direction,
                    actor: self.player_key.unwrap(),
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

        if !from.is_container_coord()
            && !from.is_inventory_coord()
            && !player_pos.is_adjacent(&from)
        {
            info!("not adjacent");
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::MoveItemDenied,
                ))
                .await?;
            return Ok(());
        }

        let source = retrieve_item(
            &map,
            &from,
            item_id,
            stack_index,
            &self.containers,
            self.player_key.unwrap(),
        );
        let Some((item, source_placement)) = source else {
            info!("cant retrieve item");
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::MoveItemDenied,
                ))
                .await?;
            return Ok(());
        };

        if item.amount < amount || item.config.has_flag(ItemFlag::Unmove) {
            info!("not enough items or item unmovable");
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::MoveItemDenied,
                ))
                .await?;
            return Ok(());
        }

        let (target_placement, target_container) = if to.is_container_coord() {
            let container_id = to.y as ContainerId;
            let container_guid = self.containers.get_global(container_id);
            if let Some(guid) = container_guid {
                let container = find_item_in_reach(&map, guid, self.player_key.unwrap());
                match container {
                    Some((container, placement)) => {
                        let slot = to.z as usize;
                        let Some(content) = &container.content else {
                            self.connection
                                .send(ConnectionCommand::SendPlayerMessage(
                                    ServerMessage::MoveItemDenied,
                                ))
                                .await?;
                            return Ok(());
                        };
                        let container = if let Some(slot_item) = content.get(slot) {
                            if slot_item.config.has_flag(ItemFlag::Container) {
                                slot_item
                            } else {
                                container
                            }
                        } else {
                            container
                        };

                        if !container.is_full()
                            && !item.config.has_flag(ItemFlag::Unmove)
                            && item.config.has_flag(ItemFlag::Take)
                            && container.config.has_flag(ItemFlag::Container)
                            && item.guid != container.guid
                        {
                            (Some(placement), Some(container.guid.clone()))
                        } else {
                            (None, None)
                        }
                    }
                    None => (None, None),
                }
            } else {
                (None, None)
            }
        } else if to.is_inventory_coord() {
            let Some(target_slot) = InventorySlot::from_id(to.y) else {
                self.connection
                    .send(ConnectionCommand::SendPlayerMessage(
                        ServerMessage::MoveItemDenied,
                    ))
                    .await?;
                return Ok(());
            };

            let ok = item
                .get_slot()
                .map(|slot| {
                    slot == target_slot
                        || (slot == InventorySlot::BothHands
                            && target_slot == InventorySlot::LeftHand)
                })
                .unwrap_or(false);
            if ok {
                (
                    Some(ItemPlacement::Inventory(
                        target_slot,
                        self.player_key.unwrap(),
                    )),
                    None,
                )
            } else {
                (None, None)
            }
        } else {
            // TODO: check if target position can be reached (unsight flag)
            if map.can_drop_item(&to) && player_pos.in_viewport(&to) {
                (Some(ItemPlacement::Map(to)), None)
            } else {
                (None, None)
            }
        };

        let Some(target_placement) = target_placement else {
            info!("invalid target");
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::MoveItemDenied,
                ))
                .await?;
            return Ok(());
        };

        let item_guid = item.guid.clone();

        self.world
            .send(WorldCommand::MoveItem {
                agent: self.player_key.unwrap(),
                from: source_placement,
                item_guid,
                amount,
                to: target_placement,
                target_container,
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

        let item = retrieve_item(
            &map,
            &position,
            item_id,
            stack_index,
            &self.containers,
            self.player_key.unwrap(),
        );
        let Some((item, placement)) = item else {
            info!("cant retrieve item");
            send_error_ack().await?;
            return Ok(());
        };

        if let ItemPlacement::Map(pos) = &placement {
            if !player_pos.is_adjacent(pos) {
                info!("not adjacent");
                send_error_ack().await?;
                return Ok(());
            }
        }

        if !item.config.has_flag(ItemFlag::Usable) {
            info!("item not usable");
            send_error_ack().await?;
            return Ok(());
        }

        self.world
            .send(WorldCommand::UseItem {
                agent: self.player_key.unwrap(),
                guid: item.guid.clone(),
                placement,
            })
            .await?;

        Ok(())
    }

    async fn handle_open_parent_container(&mut self, container_id: ContainerId) -> Result<()> {
        let container_guid = self.containers.get_global(container_id);
        if let Some(guid) = container_guid {
            let map = self.shared_map.load();
            let container = find_parent_container(&map, guid, self.player_key.unwrap());
            if let Some((parent_guid, placement)) = container {
                return self
                    .open_container(self.player_key.unwrap(), parent_guid.clone(), placement)
                    .await;
            }
        }

        Ok(())
    }

    async fn handle_change_direction(&self, facing: Facing) -> Result<()> {
        let map = self.shared_map.load();
        let agent = map
            .get_agent(self.player_key.unwrap())
            .ok_or(SessionError::NotSpawned)?;

        if agent.facing != facing {
            self.world
                .send(WorldCommand::ChangeDirection {
                    agent: self.player_key.unwrap(),
                    facing,
                })
                .await?;
        } else {
            self.actor_direction_changed(self.player_key.unwrap(), facing)
                .await?;
        }

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
                to_position,
            } => self.agent_moved(agent_key, direction, to_position).await,
            BroadcastMessage::PlayerSpawned {
                agent_key,
                position,
            } => self.player_spawned(agent_key, position).await,
            BroadcastMessage::MoveAck { agent_key } => {
                self.move_item_result(agent_key, true, None).await
            }
            BroadcastMessage::MoveDenied { agent_key, message } => {
                self.move_item_result(agent_key, false, Some(message)).await
            }
            BroadcastMessage::TileChanged { position } => self.tile_changed(position).await,
            BroadcastMessage::UseItemAck { agent_key, success } => {
                self.use_item_ack(agent_key, success).await
            }
            BroadcastMessage::OpenContainer {
                agent_key,
                guid,
                placement,
            } => self.open_container(agent_key, guid, placement).await,
            BroadcastMessage::UpdateContainer { guid, placement } => {
                self.update_container(guid, placement).await
            }
            BroadcastMessage::PlayerWalkDenied { agent_key } => self.walk_denied(agent_key).await,
            BroadcastMessage::UpdateInventorySlot { agent_key, slot } => {
                self.update_inventory_slot(agent_key, slot).await
            }
            BroadcastMessage::UpdatePlayerCapacity { agent_key } => {
                self.update_player_capacity(agent_key).await
            }
            BroadcastMessage::AgentChangedDirection { agent_key, facing } => {
                self.actor_direction_changed(agent_key, facing).await
            }
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

            let player_desc = get_player_desc(
                &map,
                self.player_key.unwrap(),
                self.agents.get_local(&self.player_key.unwrap()).unwrap(),
            );
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

    async fn drop_unreachble_containers(&mut self) -> Result<()> {
        let map = self.shared_map.load();
        let mut remove: Vec<ContainerId> = Vec::new();
        for guid in self.containers.iter_global() {
            if find_item_in_reach(&map, guid, self.player_key.unwrap()).is_none() {
                remove.push(self.containers.get_local(guid).unwrap());
            }
        }
        for id in remove {
            self.containers.remove_by_local(id);
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::ContainerClosed { container_id: id },
                ))
                .await?;
        }
        Ok(())
    }

    async fn agent_moved(
        &mut self,
        agent_key: AgentKey,
        direction: Direction,
        to_position: Position,
    ) -> Result<()> {
        if self.player_key == Some(agent_key) {
            self.drop_unreachble_containers().await?;
            let map = self.shared_map.load();
            let from_pos = to_position.clone() - direction;
            let tiles = get_map_expansion(&map, &from_pos, &direction);
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::PlayerWalkAck {
                        position: to_position,
                        tiles,
                    },
                ))
                .await?;

            Ok(())
        } else {
            Ok(())
        }
    }

    async fn walk_denied(&self, agent_key: AgentKey) -> Result<()> {
        if self.player_key == Some(agent_key) {
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::PlayerWalkDenied,
                ))
                .await?;
        }
        Ok(())
    }

    async fn move_item_result(
        &self,
        agent_key: AgentKey,
        success: bool,
        message: Option<String>,
    ) -> Result<()> {
        if self.player_key == Some(agent_key) {
            if success {
                self.connection
                    .send(ConnectionCommand::SendPlayerMessage(
                        ServerMessage::MoveItemAck,
                    ))
                    .await?;
            } else {
                if let Some(message) = message {
                    self.connection
                        .send(ConnectionCommand::SendPlayerMessage(
                            ServerMessage::TextMessage {
                                text: message,
                                message_type: TextMessageType::ActionDenied,
                            },
                        ))
                        .await?;
                }
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
        self.drop_unreachble_containers().await?;
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
        placement: ItemPlacement,
    ) -> Result<()> {
        if self.player_key != Some(agent_key) {
            return Ok(());
        }

        let map = self.shared_map.load();
        let item = match &placement {
            ItemPlacement::Map(position) => {
                let item = map.get_item_by_id(position, &guid);
                let Some(item) = item else {
                    return Err(SessionError::InvalidState.into());
                };
                item
            }
            ItemPlacement::Inventory(slot, agent_key) => {
                let Some(agent) = map.get_agent(*agent_key) else {
                    return Err(SessionError::InvalidState.into());
                };
                let Some(item) = find_item_in_slot(agent, *slot, &guid) else {
                    return Err(SessionError::InvalidState.into());
                };
                item
            }
        };

        let Some(capacity) = item.config.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Capacity(c) => Some(c),
            _ => None,
        }) else {
            return Err(SessionError::InvalidState.into());
        };
        let capacity = *capacity;
        let Some(ref content) = item.content else {
            return Err(SessionError::InvalidState.into());
        };

        let title = item.get_name().to_owned();
        let items = content
            .iter()
            .map(|i| Some((i.item_id, i.amount)))
            .collect::<Vec<Option<(ItemId, u8)>>>()
            .into_boxed_slice();
        let container_id = self.containers.get_or_insert(guid.clone());
        let has_parent = find_parent_container(&map, &guid, self.player_key.unwrap()).is_some();

        self.connection
            .send(ConnectionCommand::SendPlayerMessage(
                ServerMessage::OpenContainer {
                    container_id,
                    capacity,
                    has_parent,
                    title,
                    items,
                },
            ))
            .await?;

        Ok(())
    }

    async fn update_container(&mut self, guid: ItemGuid, placement: ItemPlacement) -> Result<()> {
        if let Some(local_id) = self.containers.get_local(&guid) {
            let map = self.shared_map.load();
            let item = match &placement {
                ItemPlacement::Map(position) => {
                    let item = map.get_item_by_id(position, &guid);
                    let Some(item) = item else {
                        return Err(SessionError::InvalidState.into());
                    };
                    item
                }
                ItemPlacement::Inventory(slot, agent_key) => {
                    let Some(agent) = map.get_agent(*agent_key) else {
                        return Err(SessionError::InvalidState.into());
                    };
                    let Some(item) = find_item_in_slot(agent, *slot, &guid) else {
                        return Err(SessionError::InvalidState.into());
                    };
                    item
                }
            };

            let Some(content) = &item.content else {
                return Err(SessionError::InvalidState.into());
            };

            let items = content
                .iter()
                .map(|i| Some((i.item_id, i.amount)))
                .collect::<Vec<Option<(ItemId, u8)>>>()
                .into_boxed_slice();

            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::UpdateContainer {
                        container_id: local_id,
                        items,
                    },
                ))
                .await?;
        }

        Ok(())
    }

    async fn update_inventory_slot(
        &mut self,
        agent_key: AgentKey,
        slot: InventorySlot,
    ) -> Result<()> {
        if self.player_key == Some(agent_key) {
            self.drop_unreachble_containers().await?;
            let map = self.shared_map.load();
            let Some(agent) = map.get_agent(agent_key) else {
                return Ok(());
            };
            let Some(player) = agent.get_player() else {
                return Ok(());
            };
            let item_id = player.inventory.get(&slot).map(|it| it.item_id);
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::IventorySlotUpdated { slot, item_id },
                ))
                .await?;
        }
        Ok(())
    }

    async fn update_player_capacity(&self, agent_key: AgentKey) -> Result<()> {
        if self.player_key == Some(agent_key) {
            let map = self.shared_map.load();
            if let Some(cap) = map.get_player(agent_key).map(|player| &player.capacity) {
                self.connection
                    .send(ConnectionCommand::SendPlayerMessage(
                        ServerMessage::PlayerCapacityUpdated {
                            cap: cap.available(),
                        },
                    ))
                    .await?;
            }
        }
        Ok(())
    }

    async fn actor_direction_changed(&self, agent_key: AgentKey, facing: Facing) -> Result<()> {
        if let Some(agent_id) = self.agents.get_local(&agent_key) {
            self.connection
                .send(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::AgentChangedDirection { agent_id, facing },
                ))
                .await?;
        }
        Ok(())
    }
}
