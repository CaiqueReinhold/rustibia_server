use thiserror::Error;
use tracing::error;

use crate::{
    constants::MAX_STACK_AMOUNT,
    entities::{
        agent::AgentKey,
        combat::WeaponType,
        items::{Item, ItemFlag, ItemGuid, ItemId, ItemRef},
        map::{GameMap, MapError, RemovedItem},
        player::InventorySlot,
        position::ItemPlacement,
    },
    game::map_query::find_item_in_placement,
};

use super::events::BroadcastMessage;

#[derive(Error, Debug)]
pub enum ItemMovementError {
    #[error("Tile position does not exist")]
    TileDoesNotExist,
    #[error("Item can't be found")]
    ItemNotInPosition,
    #[error("Container is full")]
    ContainerIsFull,
    #[error("Not enough capacity")]
    NotEnoughCap,
    #[error("Player has despawned")]
    PlayerDespawned,
    #[error("Can't equip item")]
    CannotEquip,
}

#[allow(clippy::too_many_arguments)]
fn displace_inventory_items(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    agent: AgentKey,
    slot: InventorySlot,
    source_item: &Item,
    source_placement: &ItemPlacement,
    source_slot: Option<InventorySlot>,
    source_container: Option<&(ItemGuid, usize)>,
) -> Result<(), ItemMovementError> {
    let current_item = map
        .get_player_mut(agent)
        .and_then(|player| player.inventory_mut().take_slot(&slot));

    // Displace any item currently in the slot back to the source
    // (inventory-to-inventory swaps are rejected upstream, so source is always Map)
    if let Some(current_item) = current_item
        && insert_item_at(
            broadcasts,
            map,
            current_item.clone(),
            source_container,
            source_placement,
            None,
        )
        .is_err()
    {
        let fallback = map.agent_position(agent).cloned();
        if let Some(fallback) = fallback
            && let Err(e) = insert_item_at(
                broadcasts,
                map,
                current_item.clone(),
                None,
                &ItemPlacement::Map(fallback),
                None,
            )
        {
            error!(
                "Failed to displace item on move. Agent: {:?}, Item: {:?}",
                agent, current_item
            );
            return Err(e);
        }
    }

    let is_bow_or_quiver = |it: &Item| {
        it.config
            .attr_weapon_type()
            .filter(|wt| matches!(wt, WeaponType::Bow | WeaponType::Crossbow))
            .is_some()
            || it.config.has_flag(ItemFlag::AmmoContainer)
    };
    let left_is_bow_or_quiver = map
        .get_player(agent)
        .unwrap()
        .inventory
        .get(&InventorySlot::LeftHand)
        .map(is_bow_or_quiver)
        .unwrap_or(false);
    let source_is_bow_or_quiver = is_bow_or_quiver(source_item);
    if left_is_bow_or_quiver && source_is_bow_or_quiver {
        return Ok(());
    }

    if source_slot.unwrap() == InventorySlot::BothHands {
        let player = map.get_player_mut(agent).unwrap();
        if let Some(rh_item) = player.inventory_mut().take_slot(&InventorySlot::RightHand) {
            let rh_copy = rh_item.clone();
            if let Err(e) = stow_item(broadcasts, map, agent, rh_item) {
                let player = map.get_player_mut(agent).unwrap();
                let _ = player
                    .inventory_mut()
                    .insert(InventorySlot::RightHand, None, rh_copy);
                return Err(e);
            }
            broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                agent_key: agent,
                slot: InventorySlot::RightHand,
            });
        }
    }

    let left_is_two_handed = map
        .get_player(agent)
        .unwrap()
        .inventory
        .get(&InventorySlot::LeftHand)
        .map(|it| it.get_slot().unwrap() == InventorySlot::BothHands)
        .unwrap_or(false);
    if slot == InventorySlot::RightHand && left_is_two_handed {
        let player = map.get_player_mut(agent).unwrap();
        let lh_item = player
            .inventory_mut()
            .take_slot(&InventorySlot::LeftHand)
            .unwrap();
        let lh_copy = lh_item.clone();
        if let Err(e) = stow_item(broadcasts, map, agent, lh_item) {
            let player = map.get_player_mut(agent).unwrap();
            let _ = player
                .inventory_mut()
                .insert(InventorySlot::LeftHand, None, lh_copy);
            return Err(e);
        }
        broadcasts.push(BroadcastMessage::UpdateInventorySlot {
            agent_key: agent,
            slot: InventorySlot::LeftHand,
        });
    }
    Ok(())
}

pub fn move_item(
    map: &mut GameMap,
    agent: AgentKey,
    source: ItemRef,
    amount: u8,
    to: ItemPlacement,
    target_container: Option<ItemGuid>,
) -> Vec<BroadcastMessage> {
    let mut broadcasts = Vec::new();

    if map.get_player(agent).is_none() {
        return broadcasts;
    }

    let Some(player_pos) = map.agent_position(agent) else {
        return broadcasts;
    };

    if let ItemPlacement::Map(pos) = &source.placement
        && !player_pos.is_adjacent(pos)
    {
        broadcasts.push(BroadcastMessage::MoveItemDenied {
            agent_key: agent,
            message: "Item is too far".to_string(),
        });
        return broadcasts;
    }

    // Validate source item: Unmove flag and stack amount.
    {
        let item = match &source.placement {
            ItemPlacement::Map(pos) => map.get_item_by_id(pos, &source.guid),
            ItemPlacement::Inventory(slot, _) => map
                .get_player(agent)
                .and_then(|p| p.inventory.get(slot))
                .and_then(|it| it.find_by_guid(&source.guid)),
        };
        if let Some(item) = item
            && (item.config.has_flag(ItemFlag::Unmove) || item.amount < amount)
        {
            broadcasts.push(BroadcastMessage::MoveItemDenied {
                agent_key: agent,
                message: "Can't move this".to_string(),
            });
            return broadcasts;
        }
    }

    // Validate target placement.
    match (&to, target_container.as_ref()) {
        (ItemPlacement::Map(pos), None) => {
            if !map.can_drop_item(pos) || !player_pos.in_viewport(pos) {
                broadcasts.push(BroadcastMessage::MoveItemDenied {
                    agent_key: agent,
                    message: "Can't drop here".to_string(),
                });
                return broadcasts;
            }
        }
        (ItemPlacement::Inventory(target_slot, _), None) => {
            let item = match &source.placement {
                ItemPlacement::Map(pos) => map.get_item_by_id(pos, &source.guid),
                ItemPlacement::Inventory(slot, _) => map
                    .get_player(agent)
                    .and_then(|p| p.inventory.get(slot))
                    .and_then(|it| it.find_by_guid(&source.guid)),
            };
            let compatible = item
                .and_then(|it| it.get_slot())
                .map(|item_slot| {
                    item_slot == *target_slot
                        || (item_slot == InventorySlot::BothHands
                            && *target_slot == InventorySlot::LeftHand)
                })
                .unwrap_or(false);
            if !compatible {
                broadcasts.push(BroadcastMessage::MoveItemDenied {
                    agent_key: agent,
                    message: "Can't equip this here".to_string(),
                });
                return broadcasts;
            }
        }
        (placement, Some(container_guid)) => {
            let item = match &source.placement {
                ItemPlacement::Map(pos) => map.get_item_by_id(pos, &source.guid),
                ItemPlacement::Inventory(slot, _) => map
                    .get_player(agent)
                    .and_then(|p| p.inventory.get(slot))
                    .and_then(|it| it.find_by_guid(&source.guid)),
            };
            let take_ok = item
                .map(|it| it.config.has_flag(ItemFlag::Take))
                .unwrap_or(false);
            let target_is_ammo_container = find_item_in_placement(
                map,
                &ItemRef {
                    guid: container_guid.clone(),
                    placement: placement.clone(),
                },
            )
            .map(|it| it.config.has_flag(ItemFlag::AmmoContainer))
            .unwrap_or(false);
            let can_drop_to_container = item
                .filter(|_| container_guid == &source.guid)
                .filter(|_| target_is_ammo_container)
                .map(|it| it.config.attr_ammo_type().is_some())
                .unwrap_or(true);
            if !take_ok && can_drop_to_container {
                broadcasts.push(BroadcastMessage::MoveItemDenied {
                    agent_key: agent,
                    message: "Can't move this".to_string(),
                });
                return broadcasts;
            }
        }
    }

    // --- Remove from source ---
    let Ok((source_item, source_index, source_container)) =
        remove_item_at(&mut broadcasts, map, &source, amount)
    else {
        broadcasts.push(BroadcastMessage::MoveItemDenied {
            agent_key: agent,
            message: "Can't move this".to_string(),
        });
        return broadcasts;
    };

    // --- Add to target ---
    let result = if let ItemPlacement::Inventory(slot, agent) = &to
        && target_container.is_none()
    {
        displace_inventory_items(
            &mut broadcasts,
            map,
            *agent,
            *slot,
            &source_item,
            &source.placement,
            source_item.get_slot(),
            source_container.as_ref(),
        )
    } else {
        Ok(())
    };

    let container = target_container.as_ref().map(|guid| (guid.clone(), 0));
    let result = result.and_then(|_| {
        insert_item_at(
            &mut broadcasts,
            map,
            source_item.clone(),
            container.as_ref(),
            &to,
            None,
        )
    });

    if let Err(error) = result {
        // Restore item to its exact source position on failure
        if let Err(e) = insert_item_at(
            &mut broadcasts,
            map,
            source_item.clone(),
            source_container.as_ref(),
            &source.placement,
            source_index,
        ) {
            error!(
                "Failed to revert item move. Item {:?} at {:?}. Error: {}",
                source_item, source.placement, e
            );
        }

        broadcasts.push(BroadcastMessage::MoveItemDenied {
            agent_key: agent,
            message: match error {
                ItemMovementError::ItemNotInPosition | ItemMovementError::TileDoesNotExist => {
                    "Can't move this".to_string()
                }
                e => e.to_string(),
            },
        });
        return broadcasts;
    }

    let source_is_equip =
        matches!(source.placement, ItemPlacement::Inventory(..)) && source_container.is_none();
    let target_is_equip = matches!(to, ItemPlacement::Inventory(..)) && target_container.is_none();

    if (source_is_equip || target_is_equip)
        && let Some(player) = map.get_player_mut(agent)
    {
        player.update_equipment_stats();
    }

    broadcasts
}

/// Into the first backpack container with room; `Err` when there is none.
pub fn stow_item(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    agent: AgentKey,
    item: Item,
) -> Result<(), ItemMovementError> {
    let container = map
        .get_player(agent)
        .and_then(|player| player.inventory.first_available_container().cloned())
        .ok_or(ItemMovementError::CannotEquip)?;

    let player = map
        .get_player_mut(agent)
        .ok_or(ItemMovementError::PlayerDespawned)?;
    player
        .inventory_mut()
        .insert(InventorySlot::Backpack, Some((&container, 0)), item)?;

    broadcasts.push(BroadcastMessage::ContainerUpdated {
        item: ItemRef {
            guid: container,
            placement: ItemPlacement::Inventory(InventorySlot::Backpack, agent),
        },
    });
    Ok(())
}

/// Puts `item` where it came from: onto a like stack in that same placement when
/// one has room, otherwise as a new entry there, otherwise at the agent's feet.
#[allow(clippy::too_many_arguments)]
pub fn return_item(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    agent: AgentKey,
    placement: &ItemPlacement,
    container: Option<&(ItemGuid, usize)>,
    index: Option<usize>,
    mut item: Item,
) -> Result<(), ItemMovementError> {
    if merge_into_like_stack(map, placement, container.map(|(guid, _)| guid), &mut item) {
        // `insert_item_at` emits its own refresh, but a merge never reaches it: a
        // flask that stacks silently stays invisible until the container is reopened.
        broadcasts.push(match (placement, container) {
            (_, Some((guid, _))) => BroadcastMessage::ContainerUpdated {
                item: ItemRef {
                    guid: guid.clone(),
                    placement: placement.clone(),
                },
            },
            (ItemPlacement::Map(pos), None) => BroadcastMessage::TileChanged {
                position: pos.clone(),
            },
            (ItemPlacement::Inventory(slot, agent_key), None) => {
                BroadcastMessage::UpdateInventorySlot {
                    agent_key: *agent_key,
                    slot: *slot,
                }
            }
        });
        if item.amount == 0 {
            return Ok(());
        }
    }

    let slot_taken = match (placement, container) {
        (ItemPlacement::Inventory(slot, agent_key), None) => map
            .get_player(*agent_key)
            .map(|player| player.inventory.get(slot).is_some())
            .unwrap_or(true),
        _ => false,
    };

    if !slot_taken
        && insert_item_at(broadcasts, map, item.clone(), container, placement, index).is_ok()
    {
        return Ok(());
    }

    let pos = map
        .agent_position(agent)
        .cloned()
        .ok_or(ItemMovementError::PlayerDespawned)?;
    insert_item_at(broadcasts, map, item, None, &ItemPlacement::Map(pos), None)
}

/// Moves as much of `item` as the cap allows onto a like stack already sitting in
/// `placement`, reducing `item.amount` by whatever it consumed. Reports whether
/// anything moved.
fn merge_into_like_stack(
    map: &mut GameMap,
    placement: &ItemPlacement,
    container: Option<&ItemGuid>,
    item: &mut Item,
) -> bool {
    let item_id = item.item_id;
    match placement {
        ItemPlacement::Map(pos) => {
            let Ok(mut items) = map.iter_items_mut(pos) else {
                return false;
            };
            let stack = match container {
                Some(guid) => items
                    .find_map(|it| it.find_by_guid_mut(guid))
                    .and_then(|c| c.content.as_mut())
                    .and_then(|content| find_like_stack(content, item_id)),
                None => items.find(|it| is_like_stack(it, item_id)),
            };
            match stack {
                Some(stack) => top_up(stack, item) > 0,
                None => false,
            }
        }
        ItemPlacement::Inventory(slot, agent_key) => {
            let unit_weight = item.config.attr_weight().unwrap_or(0);
            let Some(player) = map.get_player_mut(*agent_key) else {
                return false;
            };
            let inventory = player.inventory_mut();
            let Some(slot_item) = inventory.get_mut(slot) else {
                return false;
            };
            let stack = match container {
                Some(guid) => slot_item
                    .find_by_guid_mut(guid)
                    .and_then(|c| c.content.as_mut())
                    .and_then(|content| find_like_stack(content, item_id)),
                None => Some(slot_item).filter(|it| is_like_stack(it, item_id)),
            };
            let moved = match stack {
                Some(stack) => top_up(stack, item),
                None => 0,
            };
            if moved == 0 {
                return false;
            }
            inventory.carried_weight += unit_weight * moved as u32;
            true
        }
    }
}

fn find_like_stack(items: &mut [Item], item_id: ItemId) -> Option<&mut Item> {
    items.iter_mut().find(|it| is_like_stack(it, item_id))
}

fn is_like_stack(item: &Item, item_id: ItemId) -> bool {
    item.item_id == item_id
        && item.config.has_flag(ItemFlag::Cumulative)
        && item.amount < MAX_STACK_AMOUNT
}

fn top_up(stack: &mut Item, item: &mut Item) -> u8 {
    let moved = item.amount.min(MAX_STACK_AMOUNT - stack.amount);
    stack.amount += moved;
    item.amount -= moved;
    moved
}

pub fn insert_item_at(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    item: Item,
    container: Option<&(ItemGuid, usize)>,
    placement: &ItemPlacement,
    index: Option<usize>,
) -> Result<(), ItemMovementError> {
    match placement {
        ItemPlacement::Map(pos) => {
            match map.place_item(pos, index, container.map(|(g, i)| (g, *i)), item) {
                Ok(..) => {
                    if let Some((guid, _)) = container {
                        broadcasts.push(BroadcastMessage::ContainerUpdated {
                            item: ItemRef {
                                guid: guid.clone(),
                                placement: placement.clone(),
                            },
                        });
                    } else {
                        broadcasts.push(BroadcastMessage::TileChanged {
                            position: pos.clone(),
                        });
                    }
                }
                Err(e) => {
                    return Err(match e {
                        MapError::ContainerIsFull => ItemMovementError::ContainerIsFull,
                        MapError::EntityNotInPosition => ItemMovementError::ItemNotInPosition,
                        MapError::TileDoesNotExist => ItemMovementError::TileDoesNotExist,
                    });
                }
            }
        }
        ItemPlacement::Inventory(slot, agent) => {
            let can_carry = map
                .get_player(*agent)
                .map(|player| player.can_carry(item.total_weight()))
                .unwrap_or(false);

            if !can_carry {
                return Err(ItemMovementError::NotEnoughCap);
            }

            if let Some((c_guid, c_index)) = container.as_ref() {
                let result = map.get_player_mut(*agent).map(|player| {
                    player
                        .inventory_mut()
                        .insert(*slot, Some((c_guid, *c_index)), item)
                });
                let Some(result) = result else {
                    return Err(ItemMovementError::PlayerDespawned);
                };
                match result {
                    Ok(..) => {
                        broadcasts.push(BroadcastMessage::ContainerUpdated {
                            item: ItemRef {
                                guid: c_guid.clone(),
                                placement: placement.clone(),
                            },
                        });
                    }
                    Err(e) => return Err(e),
                }
            } else {
                match map
                    .get_player_mut(*agent)
                    .unwrap()
                    .inventory_mut()
                    .insert(*slot, None, item)
                {
                    Ok(..) => {
                        broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                            agent_key: *agent,
                            slot: *slot,
                        });
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok(())
}

pub fn remove_item_at(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    item: &ItemRef,
    amount: u8,
) -> Result<RemovedItem, ItemMovementError> {
    let removed = match &item.placement {
        ItemPlacement::Map(pos) => {
            let removed = map.remove_item_from_tile(pos, &item.guid, amount);
            match &removed {
                Some((_, Some(_), None)) => {
                    broadcasts.push(BroadcastMessage::TileChanged {
                        position: pos.clone(),
                    });
                }
                Some((_, None, Some((guid, _)))) => {
                    broadcasts.push(BroadcastMessage::ContainerUpdated {
                        item: ItemRef {
                            guid: guid.clone(),
                            placement: item.placement.clone(),
                        },
                    });
                }
                _ => (),
            };
            removed
        }
        ItemPlacement::Inventory(slot, agent_key) => {
            let removed = map
                .get_player_mut(*agent_key)
                .and_then(|player| player.inventory_mut().remove(*slot, &item.guid, amount));
            match &removed {
                Some((_, Some((guid, _)))) => {
                    broadcasts.push(BroadcastMessage::ContainerUpdated {
                        item: ItemRef {
                            guid: guid.clone(),
                            placement: item.placement.clone(),
                        },
                    });
                }
                Some((_, None)) => {
                    broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                        agent_key: *agent_key,
                        slot: *slot,
                    });
                }
                None => (),
            };
            removed.map(|(i, p)| (i, None, p))
        }
    };
    removed.ok_or(ItemMovementError::ItemNotInPosition)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::Agent;
    use crate::entities::items::{ItemAttribute, ItemConfig};
    use crate::entities::map::MapTile;
    use crate::entities::position::Position;
    use crate::persistence::items::ITEM_CONFIGS;
    use crate::persistence::test_fixtures::{a_player_with_a_full_backpack, a_test_snapshot};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    fn a_movable_item(weight: u32) -> Item {
        Item::new(
            Arc::new(ItemConfig::new(
                1234,
                "thing".to_string(),
                None,
                None,
                HashSet::from([ItemFlag::Take]),
                HashSet::from([
                    ItemAttribute::Weight(weight),
                    ItemAttribute::Inventory(InventorySlot::Backpack),
                ]),
            )),
            1,
        )
    }

    /// `can_drop_item` requires a `FullBank` ground item on the tile, so every tile here
    /// gets one; a bare `MapTile::new()` rejects every drop.
    fn a_ground_tile() -> MapTile {
        let mut tile = MapTile::new();
        tile.push_item(Item::new(
            Arc::new(ItemConfig::new(
                1,
                "ground".to_string(),
                None,
                None,
                HashSet::from([ItemFlag::Ground, ItemFlag::FullBank]),
                HashSet::new(),
            )),
            1,
        ));
        tile
    }

    /// Player at (10,10) with `item` lying on the adjacent tile (11,10), and a free
    /// tile at (12,10) to move it to.
    fn a_player_beside(item: Item) -> (GameMap, AgentKey, Position, Position, ItemGuid) {
        let (here, source, target) = (
            Position::new(10, 10, 7),
            Position::new(11, 10, 7),
            Position::new(12, 10, 7),
        );
        let guid = item.guid.clone();
        let mut map = GameMap::new();
        map.insert_tile(here.clone(), a_ground_tile());
        let mut source_tile = a_ground_tile();
        source_tile.push_item(item);
        map.insert_tile(source.clone(), source_tile);
        map.insert_tile(target.clone(), a_ground_tile());
        let agent = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &here)
            .unwrap();
        (map, agent, source, target, guid)
    }

    #[test]
    fn a_move_between_tiles_does_not_copy_the_player() {
        let (mut map, agent, source, target, guid) = a_player_beside(a_movable_item(100));
        let before = map.clone();

        move_item(
            &mut map,
            agent,
            ItemRef {
                guid: guid.clone(),
                placement: ItemPlacement::Map(source),
            },
            1,
            ItemPlacement::Map(target.clone()),
            None,
        );

        assert!(map.get_item_by_id(&target, &guid).is_some());
        assert!(std::ptr::eq(
            map.get_player(agent).unwrap(),
            before.get_player(agent).unwrap()
        ));
    }

    #[test]
    fn a_move_into_the_inventory_copies_the_player_and_refreshes_capacity() {
        let (mut map, agent, source, _, guid) = a_player_beside(a_movable_item(100));
        let before = map.clone();
        let available_before = before.get_player(agent).unwrap().capacity_available();

        move_item(
            &mut map,
            agent,
            ItemRef {
                guid,
                placement: ItemPlacement::Map(source),
            },
            1,
            ItemPlacement::Inventory(InventorySlot::Backpack, agent),
            None,
        );

        assert_eq!(
            map.get_player(agent).unwrap().capacity_available(),
            available_before - 100
        );
        assert!(!std::ptr::eq(
            map.get_player(agent).unwrap(),
            before.get_player(agent).unwrap()
        ));
    }

    fn an_armoured_helmet() -> Item {
        Item::new(
            Arc::new(ItemConfig::new(
                4321,
                "helmet".to_string(),
                None,
                None,
                HashSet::from([ItemFlag::Take]),
                HashSet::from([
                    ItemAttribute::Weight(100),
                    ItemAttribute::Inventory(InventorySlot::Head),
                    ItemAttribute::Armor(8),
                ]),
            )),
            1,
        )
    }

    /// Both directions, because only one of them broke: the equip half passed while the
    /// un-equip half silently kept the armour, leaving a player protected by a helmet
    /// lying on the floor.
    #[test]
    fn equipping_and_unequipping_track_the_armour_total() {
        let (mut map, agent, source, target, guid) = a_player_beside(an_armoured_helmet());
        assert_eq!(map.get_player(agent).unwrap().armor, 0);

        move_item(
            &mut map,
            agent,
            ItemRef {
                guid: guid.clone(),
                placement: ItemPlacement::Map(source),
            },
            1,
            ItemPlacement::Inventory(InventorySlot::Head, agent),
            None,
        );

        assert_eq!(
            map.get_player(agent).unwrap().armor,
            8,
            "equipping should count it"
        );

        move_item(
            &mut map,
            agent,
            ItemRef {
                guid,
                placement: ItemPlacement::Inventory(InventorySlot::Head, agent),
            },
            1,
            ItemPlacement::Map(target),
            None,
        );

        assert_eq!(
            map.get_player(agent).unwrap().armor,
            0,
            "unequipping should drop it"
        );
    }

    #[test]
    fn a_stowed_item_goes_into_the_first_free_container() {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let agent = map
            .insert_agent(
                Agent::from_player(a_player_with_a_full_backpack(1, 1)),
                &pos,
            )
            .unwrap();

        let mut broadcasts = Vec::new();
        let item = Item::new(ITEM_CONFIGS.get(&283).unwrap().clone(), 1);
        let guid = item.guid.clone();

        stow_item(&mut broadcasts, &mut map, agent, item).unwrap();

        assert!(
            map.get_player(agent)
                .unwrap()
                .inventory
                .slots()
                .values()
                .any(|slot| slot.find_by_guid(&guid).is_some()),
            "the item is not anywhere in the inventory"
        );
        assert!(map.get_top_item(&pos).is_none(), "it was dropped instead");
    }

    /// `a_test_snapshot` carries no backpack, so there is nowhere to stow anything.
    /// The refusal is the point: the caller, not this helper, decides what to do next.
    #[test]
    fn a_stowed_item_is_refused_when_there_is_nowhere_to_put_it() {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let agent = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &pos)
            .unwrap();

        let mut broadcasts = Vec::new();
        let item = Item::new(ITEM_CONFIGS.get(&283).unwrap().clone(), 1);

        assert!(matches!(
            stow_item(&mut broadcasts, &mut map, agent, item),
            Err(ItemMovementError::CannotEquip)
        ));
        assert!(map.get_top_item(&pos).is_none(), "it was dropped instead");
        assert!(broadcasts.is_empty());
    }

    fn a_backpack_with(capacity: u8, items: Vec<Item>) -> Item {
        let mut backpack = Item::new(
            Arc::new(ItemConfig::new(
                1988,
                "backpack".to_string(),
                None,
                None,
                HashSet::from([ItemFlag::Container, ItemFlag::Take]),
                HashSet::from([ItemAttribute::Capacity(capacity), ItemAttribute::Weight(10)]),
            )),
            1,
        );
        backpack.content.as_mut().unwrap().extend(items);
        backpack
    }

    /// A container with a single occupied slot: `first_available_container` finds
    /// nothing, so nothing can be stowed.
    fn a_stuffed_backpack() -> Item {
        a_backpack_with(1, vec![a_movable_item(1)])
    }

    fn a_flask(amount: u8) -> Item {
        Item::new(ITEM_CONFIGS.get(&283).unwrap().clone(), amount)
    }

    fn an_item_for(slot: InventorySlot) -> Item {
        Item::new(
            Arc::new(ItemConfig::new(
                2509,
                "gear".to_string(),
                None,
                None,
                HashSet::from([ItemFlag::Take]),
                HashSet::from([ItemAttribute::Weight(100), ItemAttribute::Inventory(slot)]),
            )),
            1,
        )
    }

    /// The off-hand item has nowhere to go, so the equip is refused. Serving it by
    /// dropping the shield on the floor, where anyone can take it, loses the item.
    #[test]
    fn a_two_handed_equip_with_no_room_to_stow_the_off_hand_is_refused() {
        let (here, source) = (Position::new(10, 10, 7), Position::new(11, 10, 7));
        let mut map = GameMap::new();
        map.insert_tile(here.clone(), a_ground_tile());
        let mut source_tile = a_ground_tile();
        let weapon = an_item_for(InventorySlot::BothHands);
        let weapon_guid = weapon.guid.clone();
        source_tile.push_item(weapon);
        map.insert_tile(source.clone(), source_tile);

        let shield = an_item_for(InventorySlot::RightHand);
        let shield_guid = shield.guid.clone();
        let mut snapshot = a_test_snapshot(1, 1);
        snapshot.inventory = HashMap::from([
            (InventorySlot::Backpack, a_stuffed_backpack()),
            (InventorySlot::RightHand, shield),
        ]);
        let agent = map
            .insert_agent(Agent::from_player(snapshot), &here)
            .unwrap();

        let broadcasts = move_item(
            &mut map,
            agent,
            ItemRef {
                guid: weapon_guid.clone(),
                placement: ItemPlacement::Map(source.clone()),
            },
            1,
            ItemPlacement::Inventory(InventorySlot::LeftHand, agent),
            None,
        );

        assert!(
            broadcasts
                .iter()
                .any(|b| matches!(b, BroadcastMessage::MoveItemDenied { .. })),
            "the equip was not refused: {broadcasts:?}"
        );
        let player = map.get_player(agent).unwrap();
        assert_eq!(
            player
                .inventory
                .get(&InventorySlot::RightHand)
                .map(|it| it.guid.clone()),
            Some(shield_guid),
            "the off-hand item left the hand"
        );
        assert!(
            player.inventory.get(&InventorySlot::LeftHand).is_none(),
            "the two-hander was equipped anyway"
        );
        assert!(
            map.get_item_by_id(&source, &weapon_guid).is_some(),
            "the two-hander did not go back to the ground"
        );
        assert!(
            map.get_top_item(&here).map(|it| it.guid.clone()) != Some(weapon_guid),
            "the two-hander landed at the player's feet"
        );
    }

    /// The placement an item came from can be gone by the time it comes back -- a tile
    /// that no longer exists, say. It lands at the agent's feet rather than nowhere.
    #[test]
    fn a_returned_item_falls_to_the_agents_feet_when_its_placement_is_gone() {
        let pos = Position::new(10, 10, 7);
        let gone = Position::new(500, 500, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let agent = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &pos)
            .unwrap();

        let mut broadcasts = Vec::new();
        let item = a_flask(1);
        let guid = item.guid.clone();

        return_item(
            &mut broadcasts,
            &mut map,
            agent,
            &ItemPlacement::Map(gone),
            None,
            None,
            item,
        )
        .unwrap();

        assert_eq!(map.get_top_item(&pos).map(|i| i.guid.clone()), Some(guid));
    }

    /// Nothing else updates `carried_weight` for an item that never passed through
    /// `Inventory::insert`, so a merged stack would otherwise weigh nothing at all.
    #[test]
    fn a_stack_merged_into_the_inventory_still_counts_towards_the_carried_weight() {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let backpack = a_backpack_with(8, vec![a_flask(1)]);
        let container_guid = backpack.guid.clone();
        let mut snapshot = a_test_snapshot(1, 1);
        snapshot.inventory = HashMap::from([(InventorySlot::Backpack, backpack)]);
        let agent = map
            .insert_agent(Agent::from_player(snapshot), &pos)
            .unwrap();
        let before = map.get_player(agent).unwrap().inventory.carried_weight;

        let mut broadcasts = Vec::new();
        return_item(
            &mut broadcasts,
            &mut map,
            agent,
            &ItemPlacement::Inventory(InventorySlot::Backpack, agent),
            Some(&(container_guid, 0)),
            Some(0),
            a_flask(1),
        )
        .unwrap();

        let player = map.get_player(agent).unwrap();
        let content = player
            .inventory
            .get(&InventorySlot::Backpack)
            .unwrap()
            .content
            .as_ref()
            .unwrap();
        assert_eq!(
            content
                .iter()
                .map(|it| (it.item_id, it.amount))
                .collect::<Vec<_>>(),
            vec![(283, 2)],
            "it opened a second entry instead of merging"
        );
        assert_eq!(
            player.inventory.carried_weight,
            before + 160,
            "the merged flask weighs nothing"
        );
    }

    /// `insert_item_at` replaces whatever a bare slot holds and drops the loser, so a
    /// slot that filled up in the meantime is not a destination any more.
    #[test]
    fn a_returned_item_does_not_evict_whatever_took_its_slot() {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let helmet = an_armoured_helmet();
        let helmet_guid = helmet.guid.clone();
        let mut snapshot = a_test_snapshot(1, 1);
        snapshot.inventory = HashMap::from([(InventorySlot::Head, helmet)]);
        let agent = map
            .insert_agent(Agent::from_player(snapshot), &pos)
            .unwrap();

        let mut broadcasts = Vec::new();
        let item = a_flask(1);
        let guid = item.guid.clone();

        return_item(
            &mut broadcasts,
            &mut map,
            agent,
            &ItemPlacement::Inventory(InventorySlot::Head, agent),
            None,
            None,
            item,
        )
        .unwrap();

        assert_eq!(
            map.get_player(agent)
                .unwrap()
                .inventory
                .get(&InventorySlot::Head)
                .map(|it| it.guid.clone()),
            Some(helmet_guid),
            "the helmet was evicted"
        );
        assert_eq!(map.get_top_item(&pos).map(|i| i.guid.clone()), Some(guid));
    }
}
