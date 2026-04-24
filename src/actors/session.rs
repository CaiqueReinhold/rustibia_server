use std::sync::Arc;

use anyhow::Result;
use thiserror::Error;
use tokio::select;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tracing::error;
use tracing::info;

use super::world::WorldCommand;
use crate::actors::connection::ConnectionActorHandle;
use crate::actors::persistence::PersistenceActorHandle;
use crate::actors::player_query::get_agent_desc;
use crate::actors::player_query::get_agents_in_viewport;
use crate::actors::player_query::get_player_desc;
use crate::actors::world::WorldActorHandle;
use crate::config::CONFIG;
use crate::entities::agent::Agent;
use crate::entities::agent::Facing;
use crate::entities::items::{ContainerId, ItemAttribute, ItemFlag, ItemGuid, ItemRef};
use crate::entities::player::InventorySlot;
use crate::entities::position::ItemPlacement;
use crate::game::events::BroadcastMessage;
use crate::game::map_query::{
    find_item_in_reach, find_item_in_slot, find_parent_container, get_map_desc_on_viewport,
    get_map_expansion, get_tile, retrieve_item,
};
use crate::local_id::LocalIdMap;
use crate::messages::TextMessageType;
use crate::persistence::player::PlayerSnapshot;
use arc_swap::ArcSwap;

use crate::entities::{
    agent::AgentKey,
    items::ItemId,
    map::GameMap,
    position::{Direction, Position},
};
use crate::messages::{ClientMessage, ServerMessage};

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
    #[error("Connection is closed")]
    ConnectionClosed,
    #[error("Player logged out")]
    Logout,
}

#[derive(Clone, Debug)]
pub enum SessionCommand {
    Close,
    PlayerSpawnResult(Option<AgentKey>),
    ReceivePlayerMessage(ClientMessage),
    LogoutDenied,
}

#[derive(Clone, Debug)]
pub struct SessionActorHandle {
    tx: mpsc::Sender<SessionCommand>,
}

impl SessionActorHandle {
    pub async fn close(&self) -> Result<(), mpsc::error::SendError<SessionCommand>> {
        self.tx.send(SessionCommand::Close).await?;
        Ok(())
    }

    pub async fn spawn_result(
        &self,
        agent_key: Option<AgentKey>,
    ) -> Result<(), mpsc::error::SendError<SessionCommand>> {
        self.tx
            .send(SessionCommand::PlayerSpawnResult(agent_key))
            .await?;
        Ok(())
    }

    pub async fn receive_message(
        &self,
        msg: ClientMessage,
    ) -> Result<(), mpsc::error::SendError<SessionCommand>> {
        self.tx
            .send(SessionCommand::ReceivePlayerMessage(msg))
            .await?;
        Ok(())
    }

    pub async fn logout_denied(&self) -> Result<(), mpsc::error::SendError<SessionCommand>> {
        self.tx.send(SessionCommand::LogoutDenied).await?;
        Ok(())
    }
}

pub struct SessionActor {
    session_id: String,
    rx: mpsc::Receiver<SessionCommand>,
    brx: broadcast::Receiver<BroadcastMessage>,
    connection: ConnectionActorHandle,
    world: WorldActorHandle,
    player_key: Option<AgentKey>,
    shared_map: Arc<ArcSwap<GameMap>>,
    containers: LocalIdMap<ItemGuid>,
    agents: LocalIdMap<AgentKey>,
    persistence: PersistenceActorHandle,
    self_handle: SessionActorHandle,
    logout_pending: bool,
}

impl SessionActor {
    pub fn start(
        session_id: String,
        connection: ConnectionActorHandle,
        agent: Agent,
        world: WorldActorHandle,
        receiver: broadcast::Receiver<BroadcastMessage>,
        shared_map: Arc<ArcSwap<GameMap>>,
        persistence: PersistenceActorHandle,
    ) -> SessionActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let self_handle = SessionActorHandle { tx };

        let self_handle_clone = self_handle.clone();
        tokio::spawn(async move {
            let actor = Self {
                session_id,
                rx,
                connection,
                world,
                player_key: None,
                brx: receiver,
                shared_map,
                containers: LocalIdMap::new(),
                agents: LocalIdMap::new(),
                persistence,
                self_handle: self_handle_clone,
                logout_pending: false,
            };
            actor.run(agent).await;
        });

        self_handle
    }

    async fn save_player(&self) {
        let Some(key) = self.player_key else {
            return;
        };
        let map = self.shared_map.load();
        let Some(agent) = map.get_agent(key) else {
            return;
        };
        let Some(position) = map.agent_position(key) else {
            return;
        };
        let Some(snapshot) = agent.to_snapshot(position.clone()) else {
            return;
        };
        if let Err(e) = self.persistence.save_player(snapshot).await {
            error!(
                session = self.session_id,
                "Failed to queue player save: {e}"
            );
        }
    }

    async fn run(mut self, agent: Agent) {
        info!(session = self.session_id, "Session actor started");

        // Enter the world immediately — auth is already complete.
        if let Err(e) = self
            .world
            .send(WorldCommand::SpawnPlayer {
                player: agent,
                session: self.self_handle.clone(),
            })
            .await
        {
            error!(session = self.session_id, "Failed to spawn player: {e}");
            let _ = self.connection.close().await;
            return;
        }

        let mut save_timer = tokio::time::interval(CONFIG.save_interval);
        save_timer.tick().await; // skip the immediate first tick

        loop {
            let result = select! { biased;
                cmd = self.rx.recv() =>
                    if let Some(cmd) = cmd {
                        self.route_command(cmd).await
                    } else {
                        Err(SessionError::ConnectionClosed.into())
                    },
                msg = self.brx.recv() => self.route_broadcast(msg.unwrap()).await,
                _ = save_timer.tick() => {
                    self.save_player().await;
                    Ok(())
                }
            };
            if let Err(e) = result {
                if e.downcast_ref::<SessionError>()
                    .is_some_and(|e| matches!(e, SessionError::Logout))
                {
                    info!(session = self.session_id, "Player logged out cleanly");
                } else {
                    error!("Error on session command: {e}");
                }
                break;
            }
        }

        self.save_player().await;
        let _ = self.connection.close().await;

        if let Some(agent_key) = self.player_key {
            let delay_ticks =
                (CONFIG.player_despawn_delay.as_millis() / CONFIG.tick_duration.as_millis()) as u64;
            let _ = self
                .world
                .send_delayed(WorldCommand::DespawnPlayer { agent_key }, delay_ticks)
                .await;
        }
    }

    async fn route_command(&mut self, cmd: SessionCommand) -> Result<()> {
        info!(
            session = self.session_id,
            "Session received command: {:?}", cmd
        );
        match cmd {
            SessionCommand::Close => self.close_connection().await,
            SessionCommand::ReceivePlayerMessage(msg) => self.handle_client_message(msg).await,
            SessionCommand::PlayerSpawnResult(handle) => self.spawn_result(handle).await,
            SessionCommand::LogoutDenied => self.logout_denied().await,
        }
    }

    async fn logout_denied(&self) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: "You may not logout right now.".to_string(),
                message_type: TextMessageType::LogoutDenied,
            })
            .await?;
        Ok(())
    }

    async fn close_connection(&self) -> Result<()> {
        self.connection.close().await?;
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

    async fn pong(&self) -> Result<()> {
        self.connection.send_message(ServerMessage::Pong).await?;
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
            ClientMessage::Logout => self.handle_logout().await,
            ClientMessage::UseItemWith {
                source,
                source_item_id,
                source_index,
                target,
                target_item_id,
                target_index,
            } => {
                self.handle_use_item_with(
                    source,
                    source_item_id,
                    source_index,
                    target,
                    target_item_id,
                    target_index,
                )
                .await
            }
        }
    }

    async fn handle_logout(&mut self) -> Result<()> {
        if !self.logout_pending {
            self.logout_pending = true;
            self.world
                .send(WorldCommand::RequestLogout {
                    agent_key: self.player_key.unwrap(),
                    session: self.self_handle.clone(),
                })
                .await?;
        }
        Ok(())
    }

    async fn handle_move_player(&self, direction: Direction) -> Result<()> {
        let _ = self
            .world
            .send(WorldCommand::Walk {
                direction,
                actor: self.player_key.unwrap(),
            })
            .await;
        Ok(())
    }

    async fn handle_get_position(&self) -> Result<()> {
        let map = self.shared_map.load();
        if let Some(position) = map.agent_position(self.player_key.unwrap()) {
            self.connection
                .send_message(ServerMessage::PlayerPosition {
                    position: position.clone(),
                })
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
        stack_index: u8,
        to: Position,
    ) -> Result<()> {
        let map = self.shared_map.load();
        let player_key = self.player_key.unwrap();

        // Resolve source: Position → (item_guid, ItemPlacement).
        // Uses the session-local container map to translate container coords.
        let Some((item, source_placement)) = retrieve_item(
            &map,
            &from,
            item_id,
            stack_index,
            &self.containers,
            player_key,
        ) else {
            return Ok(());
        };
        let item_guid = item.guid.clone();

        // Resolve target: Position → (ItemPlacement, Option<container_guid>).
        let (target_placement, target_container) = if to.is_container_coord() {
            let container_id = to.y as ContainerId;
            let Some(guid) = self.containers.get_global(container_id) else {
                return Ok(());
            };
            let Some((container, placement)) = find_item_in_reach(&map, guid, player_key) else {
                return Ok(());
            };
            // If the target slot holds a container, redirect into it.
            let slot = to.z as usize;
            let effective_guid = container
                .content
                .as_ref()
                .and_then(|c| c.get(slot))
                .filter(|it| it.config.has_flag(ItemFlag::Container))
                .map(|it| it.guid.clone())
                .unwrap_or_else(|| container.guid.clone());
            (placement, Some(effective_guid))
        } else if to.is_inventory_coord() {
            let Some(target_slot) = InventorySlot::from_id(to.y) else {
                return Ok(());
            };
            (ItemPlacement::Inventory(target_slot, player_key), None)
        } else {
            (ItemPlacement::Map(to), None)
        };

        self.world
            .send(WorldCommand::MoveItem {
                agent: player_key,
                source: ItemRef {
                    guid: item_guid,
                    placement: source_placement,
                },
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
        stack_index: u8,
    ) -> Result<()> {
        let map = self.shared_map.load();
        let player_key = self.player_key.unwrap();

        let Some((item, placement)) = retrieve_item(
            &map,
            &position,
            item_id,
            stack_index,
            &self.containers,
            player_key,
        ) else {
            return Ok(());
        };

        self.world
            .send(WorldCommand::UseItem {
                agent: player_key,
                item: ItemRef {
                    guid: item.guid.clone(),
                    placement,
                },
            })
            .await?;

        Ok(())
    }

    async fn handle_use_item_with(
        &self,
        source: Position,
        source_item_id: ItemId,
        source_index: u8,
        target: Position,
        target_item_id: ItemId,
        target_index: u8,
    ) -> Result<()> {
        let map = self.shared_map.load();
        let player_key = self.player_key.unwrap();

        let Some((source_item, source_placement)) = retrieve_item(
            &map,
            &source,
            source_item_id,
            source_index,
            &self.containers,
            player_key,
        ) else {
            return Ok(());
        };

        let Some((target_item, target_placement)) = retrieve_item(
            &map,
            &target,
            target_item_id,
            target_index,
            &self.containers,
            player_key,
        ) else {
            return Ok(());
        };

        self.world
            .send(WorldCommand::UseItemWith {
                agent: player_key,
                source: ItemRef {
                    guid: source_item.guid.clone(),
                    placement: source_placement,
                },
                target: ItemRef {
                    guid: target_item.guid.clone(),
                    placement: target_placement,
                },
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
                    .open_container(
                        self.player_key.unwrap(),
                        ItemRef {
                            guid: parent_guid.clone(),
                            placement,
                        },
                    )
                    .await;
            }
        }

        Ok(())
    }

    async fn handle_change_direction(&self, facing: Facing) -> Result<()> {
        self.world
            .send(WorldCommand::ChangeDirection {
                agent: self.player_key.unwrap(),
                facing,
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
            BroadcastMessage::OpenContainer { agent_key, item } => {
                self.open_container(agent_key, item).await
            }
            BroadcastMessage::UpdateContainer { item } => self.update_container(item).await,
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
            BroadcastMessage::PlayerDespawned {
                agent_key,
                snapshot,
            } => self.player_despawned(agent_key, snapshot).await,
            BroadcastMessage::AgentTeleport {
                agent_key,
                position,
            } => self.agent_teleported(agent_key, position).await,
        }
    }

    async fn player_spawned(&mut self, agent_key: AgentKey, position: Position) -> Result<()> {
        if self.player_key == Some(agent_key) {
            let map = self.shared_map.load();

            let map_desc_floors = get_map_desc_on_viewport(&map, &position);
            for (floor, tiles) in map_desc_floors {
                self.connection
                    .send_message(ServerMessage::DescribeMap {
                        tiles,
                        center: position.clone(),
                        floor,
                    })
                    .await?;
            }

            for (key, agent, pos) in get_agents_in_viewport(&map, &position) {
                if Some(key) == self.player_key {
                    continue;
                }
                let agent_id = self.agents.get_or_insert(key);
                self.connection
                    .send_message(get_agent_desc(agent, agent_id, pos))
                    .await?;
            }

            let player_desc = get_player_desc(
                &map,
                self.player_key.unwrap(),
                self.agents.get_local(&self.player_key.unwrap()).unwrap(),
            );
            if let Some(pdesc_msg) = player_desc {
                self.connection.send_message(pdesc_msg).await?;
            } else {
                return Err(SessionError::FailedToInitialize.into());
            }
            Ok(())
        } else {
            let map = self.shared_map.load();
            let my_pos = map
                .agent_position(self.player_key.unwrap())
                .ok_or(SessionError::NotSpawned)?;

            if !my_pos.in_viewport(&position) {
                return Ok(());
            }
            let Some(agent) = map.get_agent(agent_key) else {
                return Ok(());
            };
            let agent_id = self.agents.get_or_insert(agent_key);

            self.connection
                .send_message(get_agent_desc(agent, agent_id, position))
                .await?;

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
                .send_message(ServerMessage::ContainerClosed { container_id: id })
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
                .send_message(ServerMessage::PlayerWalkAck {
                    position: to_position,
                    tiles,
                })
                .await?;

            Ok(())
        } else {
            let map = self.shared_map.load();
            let Some(my_pos) = map.agent_position(self.player_key.unwrap()) else {
                return Ok(());
            };
            let Some(agent_pos) = map.agent_position(agent_key) else {
                return Ok(());
            };

            if my_pos.in_viewport(agent_pos) {
                if let Some(agent_id) = self.agents.get_local(&agent_key) {
                    let from = agent_pos.clone() - direction;
                    self.connection
                        .send_message(ServerMessage::MoveAgent {
                            agent_id,
                            direction,
                            from,
                        })
                        .await?;
                } else {
                    let Some(agent) = map.get_agent(agent_key) else {
                        return Ok(());
                    };
                    let agent_id = self.agents.get_or_insert(agent_key);

                    self.connection
                        .send_message(get_agent_desc(agent, agent_id, agent_pos.clone()))
                        .await?;
                }
            } else if let Some(agent_id) = self.agents.get_local(&agent_key) {
                self.connection
                    .send_message(ServerMessage::RemoveAgent { agent_id })
                    .await?;
            }

            Ok(())
        }
    }

    async fn walk_denied(&self, agent_key: AgentKey) -> Result<()> {
        if self.player_key == Some(agent_key) {
            self.connection
                .send_message(ServerMessage::PlayerWalkDenied)
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
                    .send_message(ServerMessage::MoveItemAck)
                    .await?;
            } else {
                if let Some(message) = message {
                    self.connection
                        .send_message(ServerMessage::TextMessage {
                            text: message,
                            message_type: TextMessageType::ActionDenied,
                        })
                        .await?;
                }
                self.connection
                    .send_message(ServerMessage::MoveItemDenied)
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
                .send_message(ServerMessage::TileChanged {
                    position,
                    items: tile,
                })
                .await?;
        }

        Ok(())
    }

    async fn use_item_ack(&self, agent_key: AgentKey, success: bool) -> Result<()> {
        if self.player_key == Some(agent_key) {
            if !success {
                self.connection
                    .send_message(ServerMessage::TextMessage {
                        text: "Cannot use this".to_string(),
                        message_type: TextMessageType::ActionDenied,
                    })
                    .await?;
            }
            self.connection
                .send_message(ServerMessage::UseItemAck)
                .await?;
        }
        Ok(())
    }

    async fn open_container(&mut self, agent_key: AgentKey, item_ref: ItemRef) -> Result<()> {
        if self.player_key != Some(agent_key) {
            return Ok(());
        }

        let map = self.shared_map.load();
        let item = match &item_ref.placement {
            ItemPlacement::Map(position) => {
                let item = map.get_item_by_id(position, &item_ref.guid);
                let Some(item) = item else {
                    return Err(SessionError::InvalidState.into());
                };
                item
            }
            ItemPlacement::Inventory(slot, agent_key) => {
                let Some(agent) = map.get_agent(*agent_key) else {
                    return Err(SessionError::InvalidState.into());
                };
                let Some(item) = find_item_in_slot(agent, *slot, &item_ref.guid) else {
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
        let container_id = self.containers.get_or_insert(item_ref.guid.clone());
        let has_parent =
            find_parent_container(&map, &item_ref.guid, self.player_key.unwrap()).is_some();

        self.connection
            .send_message(ServerMessage::OpenContainer {
                container_id,
                capacity,
                has_parent,
                title,
                items,
            })
            .await?;

        Ok(())
    }

    async fn update_container(&mut self, item_ref: ItemRef) -> Result<()> {
        if let Some(local_id) = self.containers.get_local(&item_ref.guid) {
            let map = self.shared_map.load();
            let item = match &item_ref.placement {
                ItemPlacement::Map(position) => {
                    let item = map.get_item_by_id(position, &item_ref.guid);
                    let Some(item) = item else {
                        return Err(SessionError::InvalidState.into());
                    };
                    item
                }
                ItemPlacement::Inventory(slot, agent_key) => {
                    let Some(agent) = map.get_agent(*agent_key) else {
                        return Err(SessionError::InvalidState.into());
                    };
                    let Some(item) = find_item_in_slot(agent, *slot, &item_ref.guid) else {
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
                .send_message(ServerMessage::UpdateContainer {
                    container_id: local_id,
                    items,
                })
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
                .send_message(ServerMessage::IventorySlotUpdated { slot, item_id })
                .await?;
        }
        Ok(())
    }

    async fn update_player_capacity(&self, agent_key: AgentKey) -> Result<()> {
        if self.player_key == Some(agent_key) {
            let map = self.shared_map.load();
            if let Some(cap) = map.get_player(agent_key).map(|player| &player.capacity) {
                self.connection
                    .send_message(ServerMessage::PlayerCapacityUpdated {
                        cap: cap.available(),
                    })
                    .await?;
            }
        }
        Ok(())
    }

    async fn actor_direction_changed(&self, agent_key: AgentKey, facing: Facing) -> Result<()> {
        if let Some(agent_id) = self.agents.get_local(&agent_key) {
            self.connection
                .send_message(ServerMessage::AgentChangedDirection { agent_id, facing })
                .await?;
        }
        Ok(())
    }

    async fn player_despawned(
        &mut self,
        agent_key: AgentKey,
        snapshot: Option<Arc<PlayerSnapshot>>,
    ) -> Result<()> {
        if self.player_key == Some(agent_key) {
            if let Some(snapshot) = snapshot {
                if let Err(e) = self
                    .persistence
                    .save_player(snapshot.as_ref().clone())
                    .await
                {
                    error!(
                        session = self.session_id,
                        "Failed to save player on logout: {e}"
                    );
                }
                self.player_key = None;
                return Err(SessionError::Logout.into());
            }
            return Ok(());
        }
        if let Some(agent_id) = self.agents.get_local(&agent_key) {
            self.agents.remove_by_local(agent_id);
            self.connection
                .send_message(ServerMessage::RemoveAgent { agent_id })
                .await?;
        }
        Ok(())
    }

    async fn agent_teleported(&mut self, agent_key: AgentKey, position: Position) -> Result<()> {
        if Some(agent_key) == self.player_key {
            let map = self.shared_map.load();
            let map_desc_floors = get_map_desc_on_viewport(&map, &position);
            for (floor, tiles) in map_desc_floors {
                if floor >= position.z {
                    self.connection
                        .send_message(ServerMessage::DescribeMap {
                            tiles,
                            center: position.clone(),
                            floor,
                        })
                        .await?;
                }
            }

            self.connection
                .send_message(ServerMessage::TeleportAgent {
                    agent_id: self
                        .agents
                        .get_local(&agent_key)
                        .ok_or(SessionError::InvalidState)?,
                    position,
                })
                .await?;
        }
        Ok(())
    }
}
