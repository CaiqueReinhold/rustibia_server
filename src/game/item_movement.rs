use anyhow::Result;
use tracing::{error, info};

use crate::entities::{
    agent::AgentKey,
    inventory::InventoryError,
    items::{ItemFlag, ItemGuid},
    map::GameMap,
    player::InventorySlot,
    position::ItemPlacement,
};

use super::events::BroadcastMessage;

pub fn move_item(
    map: &mut GameMap,
    agent: AgentKey,
    from: ItemPlacement,
    item_guid: ItemGuid,
    amount: u8,
    to: ItemPlacement,
    target_container: Option<ItemGuid>,
) -> Result<Vec<BroadcastMessage>> {
    let mut broadcasts = Vec::new();

    if map.get_player(agent).is_none() {
        return Ok(broadcasts);
    }

    let player_pos = map
        .agent_position(agent)
        .ok_or_else(|| anyhow::anyhow!("agent {:?} position not found", agent))?
        .clone();

    if let ItemPlacement::Map(pos) = &from {
        if !player_pos.is_adjacent(pos) {
            broadcasts.push(BroadcastMessage::MoveDenied {
                agent_key: agent,
                message: "Item not in reach".to_string(),
            });
            return Ok(broadcasts);
        }
    }

    // Validate source item: Unmove flag and stack amount.
    {
        let item = match &from {
            ItemPlacement::Map(pos) => map.get_item_by_id(pos, &item_guid),
            ItemPlacement::Inventory(slot, _) => map
                .get_player(agent)
                .and_then(|p| p.inventory.get(slot))
                .and_then(|it| it.find_by_guid(&item_guid)),
        };
        if let Some(item) = item {
            if item.config.has_flag(ItemFlag::Unmove) || item.amount < amount {
                broadcasts.push(BroadcastMessage::MoveDenied {
                    agent_key: agent,
                    message: "Can't move this".to_string(),
                });
                return Ok(broadcasts);
            }
        }
    }

    // Validate target placement.
    match (&to, target_container.as_ref()) {
        (ItemPlacement::Map(pos), None) => {
            if !map.can_drop_item(pos) || !player_pos.in_viewport(pos) {
                broadcasts.push(BroadcastMessage::MoveDenied {
                    agent_key: agent,
                    message: "Can't drop here".to_string(),
                });
                return Ok(broadcasts);
            }
        }
        (ItemPlacement::Inventory(target_slot, _), None) => {
            let item = match &from {
                ItemPlacement::Map(pos) => map.get_item_by_id(pos, &item_guid),
                ItemPlacement::Inventory(slot, _) => map
                    .get_player(agent)
                    .and_then(|p| p.inventory.get(slot))
                    .and_then(|it| it.find_by_guid(&item_guid)),
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
                broadcasts.push(BroadcastMessage::MoveDenied {
                    agent_key: agent,
                    message: "Can't equip this here".to_string(),
                });
                return Ok(broadcasts);
            }
        }
        (_, Some(container_guid)) => {
            let item = match &from {
                ItemPlacement::Map(pos) => map.get_item_by_id(pos, &item_guid),
                ItemPlacement::Inventory(slot, _) => map
                    .get_player(agent)
                    .and_then(|p| p.inventory.get(slot))
                    .and_then(|it| it.find_by_guid(&item_guid)),
            };
            let take_ok = item
                .map(|it| it.config.has_flag(ItemFlag::Take))
                .unwrap_or(false);
            if !take_ok || container_guid == &item_guid {
                broadcasts.push(BroadcastMessage::MoveDenied {
                    agent_key: agent,
                    message: "Can't move this".to_string(),
                });
                return Ok(broadcasts);
            }
        }
    }

    // --- Remove from source ---
    let source = match &from {
        ItemPlacement::Map(pos) => {
            map.remove_item_from_tile(pos, &item_guid, amount)
                .map(|it| {
                    let change = if let Some((parent, _)) = &it.1 {
                        BroadcastMessage::UpdateContainer {
                            guid: parent.clone(),
                            placement: from.clone(),
                        }
                    } else {
                        BroadcastMessage::TileChanged {
                            position: pos.clone(),
                        }
                    };
                    (change, it)
                })
        }
        ItemPlacement::Inventory(slot, agent_key) => map
            .get_player_mut(agent)
            .and_then(|player| player.inventory.remove(*slot, &item_guid, amount))
            .map(|it| {
                let change = if let Some((parent, _)) = &it.1 {
                    BroadcastMessage::UpdateContainer {
                        guid: parent.clone(),
                        placement: from.clone(),
                    }
                } else {
                    BroadcastMessage::UpdateInventorySlot {
                        agent_key: *agent_key,
                        slot: *slot,
                    }
                };
                (change, it)
            }),
    };

    let Some((source_change, (item, parent))) = source else {
        broadcasts.push(BroadcastMessage::MoveDenied {
            agent_key: agent,
            message: "Can't move this".to_string(),
        });
        return Ok(broadcasts);
    };

    // --- Add to target ---
    // parent container info as Option<(&guid, slot)> — used for displacing and rollback
    let parent_ref = parent.as_ref().map(|(guid, slot)| (guid, *slot));

    let error_message = match &to {
        ItemPlacement::Map(pos) => {
            let container = target_container.as_ref().map(|g| (g, 0usize));
            match map.place_item(pos, container, item.clone()) {
                Ok(..) => {
                    if let Some(guid) = target_container.as_ref() {
                        broadcasts.push(BroadcastMessage::UpdateContainer {
                            guid: guid.clone(),
                            placement: to.clone(),
                        });
                    } else {
                        broadcasts.push(BroadcastMessage::TileChanged {
                            position: pos.clone(),
                        });
                    }
                    None
                }
                Err(e) => Some(e.to_string()),
            }
        }
        ItemPlacement::Inventory(slot, _) => {
            let can_carry = map
                .get_player(agent)
                .map(|player| player.can_carry(item.total_weight()))
                .unwrap_or(false);

            if !can_carry {
                Some("Not enough capacity".to_string())
            } else if let Some(target_container) = target_container.as_ref() {
                // Moving into a container within the inventory slot
                let result = map
                    .get_player_mut(agent)
                    .map(|player| {
                        player.inventory.insert(
                            *slot,
                            Some((target_container, 0)),
                            item.clone(),
                        )
                    })
                    .unwrap();
                match result {
                    Ok(..) => {
                        broadcasts.push(BroadcastMessage::UpdateContainer {
                            guid: target_container.clone(),
                            placement: to.clone(),
                        });
                        None
                    }
                    Err(e) => Some(e.to_string()),
                }
            } else {
                // Moving directly into an equipment slot
                let current_item = map
                    .get_player_mut(agent)
                    .and_then(|player| player.inventory.take_slot(slot));

                // Displace any item currently in the slot back to the source
                // (inventory-to-inventory swaps are rejected upstream, so from is always Map)
                if let Some(current_item) = current_item {
                    if let ItemPlacement::Map(pos) = &from {
                        if map.place_item(pos, parent_ref, current_item.clone()).is_err() {
                            let fallback = map.agent_position(agent).cloned();
                            if let Some(fallback) = fallback {
                                if map.place_item(&fallback, None, current_item).is_err() {
                                    error!(
                                        "Failed to displace item on move. Agent: {:?}, Item: {:?}",
                                        agent, item
                                    );
                                }
                            }
                        }
                    }
                }

                let mut error = None;
                if item.get_slot().unwrap() == InventorySlot::BothHands {
                    info!("is two handed");
                    let player = map.get_player_mut(agent).unwrap();
                    if let Some(rh_item) = player.inventory.take_slot(&InventorySlot::RightHand) {
                        info!("right hand item {:?}", rh_item);
                        if let Some(available_container) =
                            player.inventory.first_available_container().cloned()
                        {
                            if let Err(e) = player.inventory.insert(
                                InventorySlot::Backpack,
                                Some((&available_container, 0)),
                                rh_item.clone(),
                            ) {
                                let _ = player.inventory.insert(
                                    InventorySlot::RightHand,
                                    None,
                                    rh_item,
                                );
                                error = Some(e.to_string());
                            } else {
                                broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                                    agent_key: agent,
                                    slot: InventorySlot::RightHand,
                                });
                                broadcasts.push(BroadcastMessage::UpdateContainer {
                                    guid: available_container,
                                    placement: ItemPlacement::Inventory(
                                        InventorySlot::Backpack,
                                        agent,
                                    ),
                                });
                            }
                        } else {
                            let _ =
                                player.inventory.insert(InventorySlot::RightHand, None, rh_item);
                            error = Some(InventoryError::CannotEquip.to_string())
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
                if *slot == InventorySlot::RightHand && left_is_two_handed {
                    let player = map.get_player_mut(agent).unwrap();
                    let lh_item = player
                        .inventory
                        .take_slot(&InventorySlot::LeftHand)
                        .unwrap();
                    if let Some(available_container) =
                        player.inventory.first_available_container().cloned()
                    {
                        if let Err(e) = player.inventory.insert(
                            InventorySlot::Backpack,
                            Some((&available_container, 0)),
                            lh_item.clone(),
                        ) {
                            let _ =
                                player.inventory.insert(InventorySlot::LeftHand, None, lh_item);
                            error = Some(e.to_string());
                        }
                        broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                            agent_key: agent,
                            slot: InventorySlot::LeftHand,
                        });
                        broadcasts.push(BroadcastMessage::UpdateContainer {
                            guid: available_container,
                            placement: ItemPlacement::Inventory(InventorySlot::Backpack, agent),
                        });
                    } else {
                        let _ = player.inventory.insert(InventorySlot::LeftHand, None, lh_item);
                        error = Some(InventoryError::CannotEquip.to_string())
                    }
                }

                if let Some(e) = error {
                    Some(e)
                } else {
                    match map
                        .get_player_mut(agent)
                        .unwrap()
                        .inventory
                        .insert(*slot, None, item.clone())
                    {
                        Ok(..) => {
                            broadcasts.push(BroadcastMessage::UpdateInventorySlot {
                                agent_key: agent,
                                slot: *slot,
                            });
                            None
                        }
                        Err(e) => Some(e.to_string()),
                    }
                }
            }
        }
    };

    if let Some(error) = error_message {
        // Restore item to its exact source position on failure
        match &from {
            ItemPlacement::Map(pos) => {
                let _ = map.place_item(pos, parent_ref, item);
            }
            ItemPlacement::Inventory(slot, _) => {
                if let Some(player) = map.get_player_mut(agent) {
                    let _ = player.inventory.insert(*slot, parent_ref, item);
                }
            }
        }

        broadcasts.push(BroadcastMessage::MoveDenied {
            agent_key: agent,
            message: error,
        });
        return Ok(broadcasts);
    }

    let player = map.get_player_mut(agent);
    if let Some(player) = player {
        if player.capacity.current != player.inventory.carried_weight {
            player.capacity.current = player.inventory.carried_weight;
            broadcasts.push(BroadcastMessage::UpdatePlayerCapacity { agent_key: agent });
        }
    }
    broadcasts.push(BroadcastMessage::MoveAck { agent_key: agent });
    broadcasts.push(source_change);

    Ok(broadcasts)
}
