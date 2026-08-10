use std::{collections::HashMap, sync::Arc};

use thiserror::Error;
use tracing::{error, warn};

use crate::{
    actors::world::{ScheduledCommand, WorldCommand},
    entities::{
        agent::AgentKey,
        items::{Item, ItemAction, ItemConfig, ItemFlag, ItemId, ItemRef},
        map::GameMap,
        position::ItemPlacement,
    },
    game::{
        Tick,
        game_config::GAME_CONFIG,
        item_movement::{ItemMovementError, insert_item_at, remove_item_at},
    },
};

use super::{events::BroadcastMessage, map_query::find_item_in_placement};

#[derive(Error, Debug)]
pub enum ItemActionError {
    #[error("Action failed")]
    ActionFailed,
    #[error("Invalid State")]
    InvalidState,
}

pub fn decay_item(
    map: &mut GameMap,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    item_ref: ItemRef,
    current_tick: Tick,
) -> (Vec<BroadcastMessage>, Vec<ScheduledCommand>) {
    let (mut broadcasts, mut commands) = (vec![], vec![]);
    let Some(item) = find_item_in_placement(map, &item_ref) else {
        return (broadcasts, commands);
    };
    let Some((_, decay_to)) = item.get_decay() else {
        return (broadcasts, commands);
    };
    let Some(config) = item_configs.get(&decay_to) else {
        error!("Config not found for item id {decay_to}");
        return (broadcasts, commands);
    };
    let new_item = Item::new(decay_to, config.clone(), 1);
    check_decay(
        &mut commands,
        &new_item,
        item_ref.placement.clone(),
        current_tick,
    );
    let Ok((old_item, source_index, source_cointainer)) =
        remove_item_at(&mut broadcasts, map, &item_ref, 1)
    else {
        return (vec![], vec![]);
    };
    if insert_item_at(
        &mut broadcasts,
        map,
        new_item,
        source_cointainer.as_ref(),
        &item_ref.placement,
        source_index,
    )
    .is_err()
    {
        if let Err(e) = insert_item_at(
            &mut broadcasts,
            map,
            old_item.clone(),
            source_cointainer.as_ref(),
            &item_ref.placement,
            source_index,
        ) {
            error!(
                "Failed to revert item move. Item {:?} at {:?}. Error {}",
                old_item, item_ref.placement, e
            );
        }
        return (vec![], vec![]);
    };

    (broadcasts, commands)
}

fn check_decay(
    commands: &mut Vec<ScheduledCommand>,
    item: &Item,
    placement: ItemPlacement,
    current_tick: Tick,
) {
    if let Some((duration, _)) = item.get_decay() {
        commands.push(ScheduledCommand {
            at_tick: current_tick + duration,
            command: WorldCommand::DecayItem {
                item: ItemRef {
                    guid: item.guid.clone(),
                    placement,
                },
            },
        });
    }
}

pub fn use_item(
    map: &mut GameMap,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    agent_key: AgentKey,
    item_ref: ItemRef,
    current_tick: Tick,
) -> (Vec<BroadcastMessage>, Vec<ScheduledCommand>) {
    let use_item_failed = |message| {
        (
            vec![BroadcastMessage::UseItemDenied { agent_key, message }],
            vec![],
        )
    };
    if map
        .get_agent(agent_key)
        .map(|agent| agent.next_use_tick > current_tick)
        .unwrap_or(false)
    {
        return use_item_failed("Can't use that fast".to_owned());
    }

    if map
        .agent_position(agent_key)
        .filter(|player_pos| player_pos.placement_is_adjacent(&item_ref.placement))
        .is_none()
    {
        return use_item_failed("Item is too far".to_owned());
    }

    let Some(item) = find_item_in_placement(map, &item_ref) else {
        return use_item_failed("Item was not found".to_owned());
    };

    if !item.config.has_flag(ItemFlag::Usable) {
        return use_item_failed("Can't use that".to_owned());
    }

    let is_container = item.config.has_flag(ItemFlag::Container);
    let action = item.get_action();

    if is_container {
        return (
            vec![BroadcastMessage::OpenContainer {
                agent_key,
                item: item_ref,
            }],
            vec![],
        );
    } else if let Some(action) = action {
        match route_action(
            &action,
            item_configs,
            map,
            agent_key,
            &item_ref,
            current_tick,
        ) {
            Ok((action_broadcasts, scheduled_commands)) => {
                map.get_agent_mut(agent_key).unwrap().next_use_tick =
                    current_tick + GAME_CONFIG.action.use_item_cooldown_ticks;
                return (action_broadcasts, scheduled_commands);
            }
            Err(e) => {
                if let ItemActionError::InvalidState = e {
                    warn!("{e}");
                }
            }
        }
    }
    use_item_failed("Can't use that".to_owned())
}

pub fn route_action(
    action: &ItemAction,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    map: &mut GameMap,
    _agent_key: AgentKey,
    item: &ItemRef,
    current_tick: Tick,
) -> Result<(Vec<BroadcastMessage>, Vec<ScheduledCommand>), ItemActionError> {
    let mut broadcasts = Vec::new();
    let mut commands = Vec::new();
    match action {
        ItemAction::Transform { into } => transform(
            &mut broadcasts,
            &mut commands,
            map,
            item_configs,
            item,
            *into,
            current_tick,
        )?,
    };
    Ok((broadcasts, commands))
}

pub(super) fn transform(
    broadcasts: &mut Vec<BroadcastMessage>,
    commands: &mut Vec<ScheduledCommand>,
    map: &mut GameMap,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    item: &ItemRef,
    into: ItemId,
    current_tick: Tick,
) -> Result<(), ItemActionError> {
    let Ok((old_item, source_index, source_container)) = remove_item_at(broadcasts, map, item, 1)
    else {
        return Err(ItemActionError::ActionFailed);
    };

    let config = item_configs
        .get(&into)
        .unwrap_or_else(|| panic!("item config missing for transform target {into}"));
    let new_item = Item::new(into, config.clone(), 1);
    check_decay(commands, &new_item, item.placement.clone(), current_tick);

    if let Err(e) = insert_item_at(
        broadcasts,
        map,
        new_item.clone(),
        source_container.as_ref(),
        &item.placement,
        source_index,
    ) {
        let result = match e {
            ItemMovementError::NotEnoughCap
                if let ItemPlacement::Inventory(_, agent_key) = &item.placement =>
            {
                if let Some(pos) = map.agent_position(*agent_key).cloned() {
                    insert_item_at(
                        broadcasts,
                        map,
                        new_item,
                        None,
                        &ItemPlacement::Map(pos),
                        None,
                    )
                } else {
                    Err(ItemMovementError::PlayerDespawned)
                }
            }
            e => Err(e),
        };

        if result.is_err() {
            if let Err(e) = insert_item_at(
                broadcasts,
                map,
                old_item.clone(),
                source_container.as_ref(),
                &item.placement,
                source_index,
            ) {
                error!(
                    "Failed to revert item move. Item {:?} at {:?}. Error: {}",
                    old_item, item.placement, e
                );
            }

            return Err(ItemActionError::ActionFailed);
        }
    }

    Ok(())
}
