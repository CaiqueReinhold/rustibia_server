//! Items and containers: the client's item commands, the container ids they
//! resolve through, and the world's replies to them.

use anyhow::Result;

use crate::actors::player_query::client_position_to_placement;
use crate::actors::session::{SessionActor, SessionError};
use crate::actors::world::WorldCommand;
use crate::entities::agent::AgentKey;
use crate::entities::items::{ContainerId, ItemAttribute, ItemFlag, ItemId, ItemRef};
use crate::entities::player::InventorySlot;
use crate::entities::position::{ItemPlacement, Position};
use crate::game::description::get_look_description;
use crate::game::map_query::{
    find_item_in_reach, find_item_in_slot, find_parent_container, get_tile, retrieve_item,
};
use crate::messages::ServerMessage;
use crate::messages::TextMessageType;

impl SessionActor {
    pub(super) async fn handle_move_item(
        &self,
        from: Position,
        item_id: ItemId,
        amount: u8,
        stack_index: u8,
        to: Position,
    ) -> Result<()> {
        let map = self.shared_map.load();
        let player_key = self.player_key;

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
            .await;

        Ok(())
    }

    pub(super) async fn handle_use_item(
        &self,
        position: Position,
        item_id: ItemId,
        stack_index: u8,
    ) -> Result<()> {
        let map = self.shared_map.load();

        let Some((item, placement)) = retrieve_item(
            &map,
            &position,
            item_id,
            stack_index,
            &self.containers,
            self.player_key,
        ) else {
            return Ok(());
        };

        self.world
            .send(WorldCommand::UseItem {
                agent: self.player_key,
                item: ItemRef {
                    guid: item.guid.clone(),
                    placement,
                },
            })
            .await;

        Ok(())
    }

    pub(super) async fn handle_use_item_with(
        &self,
        source: Position,
        source_item_id: ItemId,
        source_index: u8,
        target: Position,
        target_item_id: ItemId,
        target_index: u8,
    ) -> Result<()> {
        let map = self.shared_map.load();

        let Some((source_item, source_placement)) = retrieve_item(
            &map,
            &source,
            source_item_id,
            source_index,
            &self.containers,
            self.player_key,
        ) else {
            return Ok(());
        };

        let Some((target_item, target_placement)) = retrieve_item(
            &map,
            &target,
            target_item_id,
            target_index,
            &self.containers,
            self.player_key,
        ) else {
            return Ok(());
        };

        self.world
            .send(WorldCommand::UseItemWith {
                agent: self.player_key,
                source: ItemRef {
                    guid: source_item.guid.clone(),
                    placement: source_placement,
                },
                target: ItemRef {
                    guid: target_item.guid.clone(),
                    placement: target_placement,
                },
            })
            .await;

        Ok(())
    }

    pub(super) fn handle_close_container(&mut self, container_id: ContainerId) -> Result<()> {
        self.containers.remove_by_local(container_id);
        Ok(())
    }

    pub(super) async fn handle_open_parent_container(
        &mut self,
        container_id: ContainerId,
    ) -> Result<()> {
        let container_guid = self.containers.get_global(container_id);
        if let Some(guid) = container_guid {
            let map = self.shared_map.load();
            let container = find_parent_container(&map, guid, self.player_key);
            if let Some((parent_guid, placement)) = container {
                return self
                    .open_container(ItemRef {
                        guid: parent_guid.clone(),
                        placement,
                    })
                    .await;
            }
        }

        Ok(())
    }

    pub(super) async fn handle_look(&self, position: Position) -> Result<()> {
        let map = self.shared_map.load();
        let player_pos = map
            .agent_position(self.player_key)
            .ok_or(SessionError::InvalidState)?;
        let Some((placement, guid)) =
            client_position_to_placement(position, &map, &self.containers, self.player_key)
        else {
            return Ok(());
        };
        let desc = get_look_description(&map, &placement, guid, player_pos);
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: desc,
                message_type: TextMessageType::Look,
            })
            .await?;
        Ok(())
    }

    pub(super) async fn move_item_denied(&self, message: String) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: message,
                message_type: TextMessageType::ActionDenied,
            })
            .await?;

        Ok(())
    }

    pub(super) async fn use_item_denied(&self, message: String) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: message,
                message_type: TextMessageType::ActionDenied,
            })
            .await?;

        Ok(())
    }

    pub(super) async fn open_container(&mut self, item_ref: ItemRef) -> Result<()> {
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
        let has_parent = find_parent_container(&map, &item_ref.guid, self.player_key).is_some();

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

    pub(super) async fn update_container(&mut self, item_ref: ItemRef) -> Result<()> {
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

    pub(super) async fn update_inventory_slot(
        &mut self,
        agent_key: AgentKey,
        slot: InventorySlot,
    ) -> Result<()> {
        self.drop_unreachable_containers().await?;
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

        Ok(())
    }

    pub(super) async fn update_player_capacity(&self, agent_key: AgentKey) -> Result<()> {
        let map = self.shared_map.load();
        if let Some(cap) = map.get_player(agent_key).map(|player| &player.capacity) {
            self.connection
                .send_message(ServerMessage::PlayerCapacityUpdated {
                    cap: cap.available(),
                })
                .await?;
        } else {
            return Err(SessionError::InvalidState.into());
        }

        Ok(())
    }

    pub(super) async fn drop_unreachable_containers(&mut self) -> Result<()> {
        let map = self.shared_map.load();
        let mut remove: Vec<ContainerId> = Vec::new();
        for guid in self.containers.iter_global() {
            if find_item_in_reach(&map, guid, self.player_key).is_none() {
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

    pub(super) async fn tile_changed(&mut self, position: Position) -> Result<()> {
        self.drop_unreachable_containers().await?;
        let map = self.shared_map.load();
        let player_pos = map
            .agent_position(self.player_key)
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
}
