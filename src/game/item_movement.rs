use thiserror::Error;
use tracing::error;

use crate::entities::{
    agent::AgentKey,
    items::{Item, ItemFlag, ItemGuid, ItemRef},
    map::{GameMap, MapError, RemovedItem},
    player::InventorySlot,
    position::ItemPlacement,
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

fn displace_inventory_items(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    agent: AgentKey,
    slot: InventorySlot,
    source_slot: Option<InventorySlot>,
    source_placement: &ItemPlacement,
    source_container: Option<&(ItemGuid, usize)>,
) -> Result<(), ItemMovementError> {
    let current_item = map
        .get_player_mut(agent)
        .and_then(|player| player.inventory.take_slot(&slot));

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

    if source_slot.unwrap() == InventorySlot::BothHands {
        let player = map.get_player_mut(agent).unwrap();
        if let Some(rh_item) = player.inventory.take_slot(&InventorySlot::RightHand) {
            if let Some(available_container) = player.inventory.first_available_container().cloned()
            {
                if let Err(e) = player.inventory.insert(
                    InventorySlot::Backpack,
                    Some((&available_container, 0)),
                    rh_item.clone(),
                ) {
                    let _ = player
                        .inventory
                        .insert(InventorySlot::RightHand, None, rh_item);
                    return Err(e);
                } else {
                    broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                        agent_key: agent,
                        slot: InventorySlot::RightHand,
                    });
                    broadcasts.push(BroadcastMessage::UpdateContainer {
                        item: ItemRef {
                            guid: available_container,
                            placement: ItemPlacement::Inventory(InventorySlot::Backpack, agent),
                        },
                    });
                }
            } else {
                let _ = player
                    .inventory
                    .insert(InventorySlot::RightHand, None, rh_item);
                return Err(ItemMovementError::CannotEquip);
            }
        }
    }

    let left_is_two_handed = map
        .get_player_mut(agent)
        .unwrap()
        .inventory
        .get(&InventorySlot::LeftHand)
        .map(|it| it.get_slot().unwrap() == InventorySlot::BothHands)
        .unwrap_or(false);
    if slot == InventorySlot::RightHand && left_is_two_handed {
        let player = map.get_player_mut(agent).unwrap();
        let lh_item = player
            .inventory
            .take_slot(&InventorySlot::LeftHand)
            .unwrap();
        if let Some(available_container) = player.inventory.first_available_container().cloned() {
            if let Err(e) = player.inventory.insert(
                InventorySlot::Backpack,
                Some((&available_container, 0)),
                lh_item.clone(),
            ) {
                let _ = player
                    .inventory
                    .insert(InventorySlot::LeftHand, None, lh_item);
                return Err(e);
            }
            broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                agent_key: agent,
                slot: InventorySlot::LeftHand,
            });
            broadcasts.push(BroadcastMessage::UpdateContainer {
                item: ItemRef {
                    guid: available_container,
                    placement: ItemPlacement::Inventory(InventorySlot::Backpack, agent),
                },
            });
        } else {
            let _ = player
                .inventory
                .insert(InventorySlot::LeftHand, None, lh_item);
            return Err(ItemMovementError::CannotEquip);
        }
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
        (_, Some(container_guid)) => {
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
            if !take_ok || container_guid == &source.guid {
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
            source_item.get_slot(),
            &source.placement,
            source_container.as_ref(),
        )
    } else {
        Ok(())
    };

    let result = result.and_then(|_| {
        insert_item_at(
            &mut broadcasts,
            map,
            source_item.clone(),
            target_container.map(|guid| (guid, 0)).as_ref(),
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

    let player = map.get_player_mut(agent);
    if let Some(player) = player
        && player.capacity.current != player.inventory.carried_weight
    {
        player.capacity.current = player.inventory.carried_weight;
        broadcasts.push(BroadcastMessage::UpdatePlayerCapacity { agent_key: agent });
    }
    broadcasts
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
                Ok(()) => {
                    if let Some((guid, _)) = container {
                        broadcasts.push(BroadcastMessage::UpdateContainer {
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
                        .inventory
                        .insert(*slot, Some((c_guid, *c_index)), item)
                });
                let Some(result) = result else {
                    return Err(ItemMovementError::PlayerDespawned);
                };
                match result {
                    Ok(..) => {
                        broadcasts.push(BroadcastMessage::UpdateContainer {
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
                    .inventory
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
                    broadcasts.push(BroadcastMessage::UpdateContainer {
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
                .and_then(|player| player.inventory.remove(*slot, &item.guid, amount));
            match &removed {
                Some((_, Some((guid, _)))) => {
                    broadcasts.push(BroadcastMessage::UpdateContainer {
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
