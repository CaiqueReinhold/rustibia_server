use std::{collections::HashMap, sync::Arc};

use thiserror::Error;
use tracing::warn;

use crate::{
    entities::{
        agent::AgentKey,
        items::{Item, ItemAction, ItemConfig, ItemFlag, ItemGuid, ItemId},
        map::GameMap,
        position::ItemPlacement,
    },
};

use super::events::BroadcastMessage;

#[derive(Error, Debug)]
pub enum ItemActionError {
    #[error("Action failed")]
    ActionFailed,
    #[error("Invalid State")]
    InvalidState,
}

pub fn use_item(
    map: &mut GameMap,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    agent_key: AgentKey,
    guid: ItemGuid,
    placement: ItemPlacement,
) -> Vec<BroadcastMessage> {
    let item = match &placement {
        ItemPlacement::Map(item_pos) => {
            if map
                .agent_position(agent_key)
                .filter(|player_pos| player_pos.is_adjacent(item_pos))
                .is_none()
            {
                return vec![BroadcastMessage::UseItemAck {
                    agent_key,
                    success: false,
                }];
            }
            map.get_item_by_id(item_pos, &guid)
        }
        ItemPlacement::Inventory(slot, inv_agent_key) => map
            .get_player(*inv_agent_key)
            .and_then(|player| player.inventory.get(slot))
            .filter(|item| item.guid == guid),
    };

    let Some(item) = item else {
        return vec![BroadcastMessage::UseItemAck {
            agent_key,
            success: false,
        }];
    };

    if !item.config.has_flag(ItemFlag::Usable) {
        return vec![BroadcastMessage::UseItemAck {
            agent_key,
            success: false,
        }];
    }

    let is_container = item.config.has_flag(ItemFlag::Container);
    let action = item.get_action();

    if is_container {
        vec![
            BroadcastMessage::UseItemAck {
                agent_key,
                success: true,
            },
            BroadcastMessage::OpenContainer {
                agent_key,
                guid,
                placement,
            },
        ]
    } else if let Some(action) = action {
        match route_action(&action, item_configs, map, agent_key, &placement, &guid) {
            Ok(mut action_broadcasts) => {
                action_broadcasts.push(BroadcastMessage::UseItemAck {
                    agent_key,
                    success: true,
                });
                action_broadcasts
            }
            Err(e) => {
                if let ItemActionError::InvalidState = e {
                    warn!("{e}");
                }
                vec![BroadcastMessage::UseItemAck {
                    agent_key,
                    success: false,
                }]
            }
        }
    } else {
        vec![BroadcastMessage::UseItemAck {
            agent_key,
            success: false,
        }]
    }
}

pub fn route_action(
    action: &ItemAction,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    map: &mut GameMap,
    _agent_key: AgentKey,
    placement: &ItemPlacement,
    guid: &ItemGuid,
) -> Result<Vec<BroadcastMessage>, ItemActionError> {
    let mut broadcasts = Vec::new();
    match action {
        ItemAction::Transform { into } => {
            transform(&mut broadcasts, map, item_configs, placement, guid, *into)?
        }
    };
    Ok(broadcasts)
}

fn transform(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    placement: &ItemPlacement,
    guid: &ItemGuid,
    into: ItemId,
) -> Result<(), ItemActionError> {
    let old_item = match placement {
        ItemPlacement::Map(pos) => map.remove_item_from_tile(pos, guid, 1),
        ItemPlacement::Inventory(slot, agent_key) => map
            .get_player_mut(*agent_key)
            .and_then(|player| player.inventory.remove(*slot, guid, 1)),
    };

    if let Some((old_item, container)) = old_item {
        let config = item_configs
            .get(&into)
            .unwrap_or_else(|| panic!("item config missing for transform target {into}"));
        let new_item = Item::new(into, config.clone(), 1);
        match placement {
            ItemPlacement::Map(pos) => {
                let container = container.as_ref().map(|(g, i)| (g, *i));
                if map.place_item(pos, container, new_item).is_err() {
                    let _ = map.place_item(pos, container, old_item);
                    return Err(ItemActionError::ActionFailed);
                }
                broadcasts.push(BroadcastMessage::TileChanged {
                    position: pos.clone(),
                });
            }
            ItemPlacement::Inventory(slot, agent_key) => {
                let container = container.as_ref().map(|(g, i)| (g, *i));
                let can_carry = map
                    .get_player(*agent_key)
                    .map(|player| player.can_carry(new_item.total_weight()));
                if let Some(can_carry) = can_carry {
                    if can_carry {
                        if let Some(player) = map.get_player_mut(*agent_key) {
                            if player.inventory.insert(*slot, container, new_item).is_err() {
                                let _ = player.inventory.insert(*slot, container, old_item);
                                return Err(ItemActionError::ActionFailed);
                            }
                            if let Some((guid, _)) = container {
                                broadcasts.push(BroadcastMessage::UpdateContainer {
                                    guid: guid.clone(),
                                    placement: placement.clone(),
                                })
                            } else {
                                broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                                    agent_key: *agent_key,
                                    slot: *slot,
                                });
                            }
                        }
                    } else if let Some(pos) = map.agent_position(*agent_key).cloned() {
                        if map.place_item(&pos, None, new_item).is_err() {
                            let _ = map.get_player_mut(*agent_key).and_then(|player| {
                                player.inventory.insert(*slot, container, old_item).ok()
                            });
                            return Err(ItemActionError::ActionFailed);
                        }
                        broadcasts.push(BroadcastMessage::TileChanged { position: pos });
                    }
                } else {
                    return Err(ItemActionError::InvalidState);
                }
            }
        };
    } else {
        return Err(ItemActionError::ActionFailed);
    }

    Ok(())
}
