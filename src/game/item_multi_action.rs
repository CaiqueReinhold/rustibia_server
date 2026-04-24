use std::{collections::HashMap, sync::Arc};

use tracing::warn;

use crate::{
    actors::world::ScheduledCommand,
    entities::{
        agent::AgentKey,
        items::{ItemConfig, ItemFlag, ItemId, ItemMultiAction, ItemRef},
        map::GameMap,
        position::{ItemPlacement, Position},
    },
    game::{
        Tick,
        events::BroadcastMessage,
        game_config::GAME_CONFIG,
        item_action::{ItemActionError, transform},
        item_movement::{insert_item_at, remove_item_at},
        map_query::find_item_in_placement,
    },
};

pub fn use_item_with(
    map: &mut GameMap,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    agent_key: AgentKey,
    source: ItemRef,
    target: ItemRef,
    current_tick: Tick,
) -> (Vec<BroadcastMessage>, Vec<ScheduledCommand>) {
    let use_item_failed = || {
        (
            vec![BroadcastMessage::UseItemAck {
                agent_key,
                success: false,
            }],
            vec![],
        )
    };

    if map
        .get_agent(agent_key)
        .map(|agent| agent.next_use_tick > current_tick)
        .unwrap_or(false)
    {
        return use_item_failed();
    }

    let source_item = find_item_in_placement(map, &source);
    let Some(source_item) = source_item else {
        return use_item_failed();
    };

    if map
        .agent_position(agent_key)
        .filter(|player_pos| player_pos.placement_is_adjacent(&source.placement))
        .is_none()
    {
        return use_item_failed();
    }

    if !source_item.config.has_flag(ItemFlag::Usable) {
        return use_item_failed();
    }

    if find_item_in_placement(map, &target).is_none() {
        return use_item_failed();
    };

    let action = source_item.get_multi_action();
    if let Some(action) = action {
        match route_multi_action(
            &action,
            item_configs,
            map,
            agent_key,
            &source,
            &target,
            current_tick,
        ) {
            Ok((mut action_broadcasts, scheduled_commands)) => {
                map.get_agent_mut(agent_key).unwrap().next_use_tick =
                    current_tick + GAME_CONFIG.action.use_item_cooldown_ticks;
                action_broadcasts.push(BroadcastMessage::UseItemAck {
                    agent_key,
                    success: true,
                });
                (action_broadcasts, scheduled_commands)
            }
            Err(e) => {
                if let ItemActionError::InvalidState = e {
                    warn!("{e}");
                }
                use_item_failed()
            }
        }
    } else {
        use_item_failed()
    }
}

fn route_multi_action(
    action: &ItemMultiAction,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    map: &mut GameMap,
    agent_key: AgentKey,
    _source: &ItemRef,
    target: &ItemRef,
    current_tick: Tick,
) -> Result<(Vec<BroadcastMessage>, Vec<ScheduledCommand>), ItemActionError> {
    let mut broadcasts = Vec::new();
    let mut commands = Vec::new();
    match action {
        ItemMultiAction::Shovel => shovel(
            &mut broadcasts,
            &mut commands,
            map,
            item_configs,
            target,
            current_tick,
        )?,
        ItemMultiAction::Rope => rope(&mut broadcasts, map, agent_key, target)?,
    };
    Ok((broadcasts, commands))
}

fn shovel(
    broadcasts: &mut Vec<BroadcastMessage>,
    commands: &mut Vec<ScheduledCommand>,
    map: &mut GameMap,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    target: &ItemRef,
    current_tick: Tick,
) -> Result<(), ItemActionError> {
    let target_item = find_item_in_placement(map, target).unwrap();
    if !GAME_CONFIG
        .multi_action
        .diggable_ids
        .contains(&target_item.item_id)
    {
        return Err(ItemActionError::ActionFailed);
    }
    transform(
        broadcasts,
        commands,
        map,
        item_configs,
        target,
        target_item.item_id + 1,
        current_tick,
    )
}

fn firt_available_position_up(
    map: &GameMap,
    pos: &Position,
    agent_key: AgentKey,
) -> Option<Position> {
    for y in [pos.y - 1, pos.y + 1] {
        for x in [pos.x - 1, pos.x + 1] {
            let try_pos = Position::new(x, y, pos.z - 1);
            if map.can_move(&try_pos, agent_key) {
                return Some(try_pos);
            }
        }
    }
    None
}

fn rope(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    agent_key: AgentKey,
    target: &ItemRef,
) -> Result<(), ItemActionError> {
    let target_item = find_item_in_placement(map, target).unwrap();
    let pos = match &target.placement {
        ItemPlacement::Map(pos) => pos,
        ItemPlacement::Inventory(..) => return Err(ItemActionError::ActionFailed),
    };
    let Some(target_pos) = firt_available_position_up(map, pos, agent_key) else {
        return Err(ItemActionError::InvalidState);
    };

    if GAME_CONFIG
        .multi_action
        .rope_spot_ids
        .contains(&target_item.item_id)
    {
        map.move_agent(agent_key, &target_pos)
            .map_err(|_| ItemActionError::ActionFailed)?;
        broadcasts.push(BroadcastMessage::AgentTeleport {
            agent_key,
            position: target_pos,
        });
        return Ok(());
    } else if GAME_CONFIG
        .multi_action
        .opened_hole_ids
        .contains(&target_item.item_id)
    {
        let down = Position::new(pos.x, pos.y, pos.z + 1);
        if let Ok(last_agent) = map
            .get_agents_at(&down)
            .map(|mut agents_iter| agents_iter.next().cloned())
            && let Some(last_agent) = last_agent
        {
            if map
                .move_agent(last_agent, &target_pos)
                .map(|()| {
                    broadcasts.push(BroadcastMessage::AgentTeleport {
                        agent_key: last_agent,
                        position: target_pos,
                    });
                })
                .is_err()
            {
                return Err(ItemActionError::ActionFailed);
            }
            return Ok(());
        } else if let Some(top_item) = map.get_top_item(&down) {
            if remove_item_at(
                broadcasts,
                map,
                &ItemRef {
                    guid: top_item.guid.clone(),
                    placement: ItemPlacement::Map(down),
                },
                top_item.amount,
            )
            .and_then(|(removed_item, index, container)| {
                insert_item_at(
                    broadcasts,
                    map,
                    removed_item,
                    container.as_ref(),
                    &ItemPlacement::Map(target_pos),
                    index,
                )
            })
            .is_err()
            {
                return Err(ItemActionError::ActionFailed);
            }
            return Ok(());
        }
    }

    Err(ItemActionError::ActionFailed)
}
