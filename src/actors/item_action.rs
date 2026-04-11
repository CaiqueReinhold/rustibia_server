use std::{collections::HashMap, sync::Arc};

use thiserror::Error;

use crate::{
    actors::world::BroadcastMessage,
    entities::{
        agent::AgentKey,
        items::{Item, ItemAction, ItemConfig, ItemGuid, ItemId},
        map::GameMap,
        position::ItemPlacement,
    },
};

#[derive(Error, Debug)]
pub enum ItemActionError {
    #[error("Action failed")]
    ActionFailed,
    #[error("Invalid State")]
    InvalidState,
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
