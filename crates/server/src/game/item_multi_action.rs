use tracing::warn;

use crate::{
    actors::world::ScheduledCommand,
    entities::{
        agent::AgentKey,
        items::{ItemFlag, ItemMultiAction, ItemRef},
        map::GameMap,
        position::{ItemPlacement, Position},
    },
    game::{
        Tick,
        config::GAME_CONFIG,
        events::BroadcastMessage,
        item_action::{ItemActionError, transform},
        item_movement::{insert_item_at, remove_item_at},
        map_query::find_item_in_placement,
    },
};

pub fn use_item_with(
    map: &mut GameMap,
    agent_key: AgentKey,
    source: ItemRef,
    target: ItemRef,
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

    let source_item = find_item_in_placement(map, &source);
    let Some(source_item) = source_item else {
        return use_item_failed("Item was not found".to_owned());
    };

    if map
        .agent_position(agent_key)
        .filter(|player_pos| player_pos.placement_is_adjacent(&source.placement))
        .is_none()
    {
        return use_item_failed("Item is too fat".to_owned());
    }

    if !source_item.config.has_flag(ItemFlag::Usable) {
        return use_item_failed("Can't use that".to_owned());
    }

    if find_item_in_placement(map, &target).is_none() {
        return use_item_failed("Item was not found".to_owned());
    };

    let action = GAME_CONFIG.multi_action.tool_action(source_item.item_id);
    if let Some(action) = action {
        match route_multi_action(&action, map, agent_key, &source, &target, current_tick) {
            Ok((action_broadcasts, scheduled_commands)) => {
                map.get_agent_mut(agent_key).unwrap().next_use_tick =
                    current_tick + GAME_CONFIG.action.use_item_cooldown_ticks;

                (action_broadcasts, scheduled_commands)
            }
            Err(e) => {
                if let ItemActionError::InvalidState = e {
                    warn!("{e}");
                }
                use_item_failed("Can't use that".to_owned())
            }
        }
    } else {
        use_item_failed("Can't use that".to_owned())
    }
}

fn route_multi_action(
    action: &ItemMultiAction,
    map: &mut GameMap,
    agent_key: AgentKey,
    _source: &ItemRef,
    target: &ItemRef,
    current_tick: Tick,
) -> Result<(Vec<BroadcastMessage>, Vec<ScheduledCommand>), ItemActionError> {
    let mut broadcasts = Vec::new();
    let mut commands = Vec::new();
    match action {
        ItemMultiAction::Shovel => {
            shovel(&mut broadcasts, &mut commands, map, target, current_tick)?
        }
        ItemMultiAction::Rope => rope(&mut broadcasts, map, agent_key, target)?,
    };
    Ok((broadcasts, commands))
}

fn shovel(
    broadcasts: &mut Vec<BroadcastMessage>,
    commands: &mut Vec<ScheduledCommand>,
    map: &mut GameMap,
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
        target,
        target_item.item_id + 1,
        current_tick,
    )
}

fn first_available_position_up(
    map: &GameMap,
    pos: &Position,
    agent_key: AgentKey,
) -> Option<Position> {
    [
        Position::new(pos.x, pos.y.saturating_sub(1), pos.z - 1),
        Position::new(pos.x, pos.y.saturating_add(1), pos.z - 1),
        Position::new(pos.x.saturating_sub(1), pos.y, pos.z - 1),
        Position::new(pos.x.saturating_add(1), pos.y, pos.z - 1),
    ]
    .iter()
    .find(|try_pos| map.can_move(try_pos, agent_key))
    .cloned()
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
    let Some(target_pos) = first_available_position_up(map, pos, agent_key) else {
        return Err(ItemActionError::InvalidState);
    };

    if GAME_CONFIG
        .multi_action
        .rope_spot_ids
        .contains(&target_item.item_id)
    {
        map.move_agent(agent_key, &target_pos)
            .map_err(|_| ItemActionError::ActionFailed)?;
        broadcasts.push(BroadcastMessage::AgentTeleported {
            agent_key,
            from_position: pos.clone(),
            to_position: target_pos,
        });
        return Ok(());
    } else if GAME_CONFIG
        .multi_action
        .opened_hole_ids
        .contains(&target_item.item_id)
    {
        let down = Position::new(pos.x, pos.y, pos.z + 1);
        if let Ok(last_agent) = map
            .iter_agents_at(&down)
            .map(|mut agents_iter| agents_iter.next().cloned())
            && let Some(last_agent) = last_agent
        {
            if map
                .move_agent(last_agent, &target_pos)
                .map(|()| {
                    broadcasts.push(BroadcastMessage::AgentTeleported {
                        agent_key: last_agent,
                        from_position: pos.clone(),
                        to_position: target_pos,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{
        agent::Agent,
        items::{Item, ItemId},
        map::MapTile,
    };
    use crate::persistence::items::ITEM_CONFIGS;
    use crate::persistence::test_fixtures::a_test_snapshot;

    fn an_item(id: ItemId) -> Item {
        Item::new(
            ITEM_CONFIGS
                .get(&id)
                .unwrap_or_else(|| panic!("item {id} missing from the shipped catalogue"))
                .clone(),
            1,
        )
    }

    fn a_tile_with(item: Item) -> MapTile {
        let mut tile = MapTile::new();
        tile.push_item(item);
        tile
    }

    /// A configured id that is absent from the catalogue, or that lost `usable`, reaches
    /// `use_item_with` and is refused with "Can't use that" and no error anywhere.
    #[test]
    fn every_configured_tool_exists_and_is_usable() {
        let config = &GAME_CONFIG.multi_action;
        assert!(!config.shovel_ids.is_empty(), "no shovel is configured");
        assert!(!config.rope_ids.is_empty(), "no rope is configured");

        for (ids, expected) in [
            (&config.shovel_ids, ItemMultiAction::Shovel),
            (&config.rope_ids, ItemMultiAction::Rope),
        ] {
            for id in ids {
                let item = ITEM_CONFIGS
                    .get(id)
                    .unwrap_or_else(|| panic!("tool {id} is not in the catalogue"));
                assert!(
                    item.has_flag(ItemFlag::Usable),
                    "tool {id} ({}) lost its usable flag",
                    item.name
                );
                assert_eq!(config.tool_action(*id), Some(expected.clone()));
            }
        }
    }

    /// `usable + multiuse` is what separates the tools from the rope bridges, railings and
    /// decorations sharing their names; a tool the config omits is inert, and silently so.
    #[test]
    fn every_tool_in_the_catalogue_is_configured() {
        let config = &GAME_CONFIG.multi_action;
        let mut unconfigured = Vec::new();

        for (id, item) in ITEM_CONFIGS.iter() {
            let is_tool = item.has_flag(ItemFlag::Usable) && item.has_flag(ItemFlag::Multiuse);
            let named = item.name.contains("shovel") || item.name.contains("rope");
            if is_tool && named && config.tool_action(*id).is_none() {
                unconfigured.push((*id, item.name.clone()));
            }
        }

        unconfigured.sort();
        assert!(
            unconfigured.is_empty(),
            "usable multiuse tools missing from game_conf.yaml: {unconfigured:?}"
        );
        assert_eq!(config.shovel_ids.len(), 6);
        assert_eq!(config.rope_ids.len(), 4);
    }

    /// `shovel` digs into `item_id + 1`, and `transform` refuses an id the catalogue does
    /// not carry, so an unpaired diggable is a dig that silently does nothing.
    #[test]
    fn every_diggable_digs_into_an_existing_item() {
        for id in &GAME_CONFIG.multi_action.diggable_ids {
            let into = id + 1;
            assert!(
                ITEM_CONFIGS.contains_key(&into),
                "diggable {id} digs into {into}, which is not in the catalogue"
            );
        }
    }

    /// Reports whether the action was denied, and what is left on the target tile.
    fn use_tool_on(tool_id: ItemId, target_id: ItemId) -> (bool, Option<Item>) {
        let (here, there) = (Position::new(10, 10, 7), Position::new(10, 11, 7));
        let mut map = GameMap::new();

        let (tool, target) = (an_item(tool_id), an_item(target_id));
        let (tool_guid, target_guid) = (tool.guid.clone(), target.guid.clone());
        map.insert_tile(here.clone(), a_tile_with(tool));
        map.insert_tile(there.clone(), a_tile_with(target));
        let agent = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &here)
            .unwrap();

        let (broadcasts, _) = use_item_with(
            &mut map,
            agent,
            ItemRef {
                guid: tool_guid,
                placement: ItemPlacement::Map(here),
            },
            ItemRef {
                guid: target_guid,
                placement: ItemPlacement::Map(there.clone()),
            },
            0,
        );

        let denied = broadcasts
            .iter()
            .any(|b| matches!(b, BroadcastMessage::UseItemDenied { .. }));
        (denied, map.get_top_item(&there).cloned())
    }

    #[test]
    fn every_shovel_digs_every_diggable_into_a_hole() {
        let config = &GAME_CONFIG.multi_action;
        for shovel_id in &config.shovel_ids {
            for diggable_id in &config.diggable_ids {
                let (denied, dug) = use_tool_on(*shovel_id, *diggable_id);
                let name = &ITEM_CONFIGS[shovel_id].name;

                assert!(!denied, "{name} ({shovel_id}) could not dig {diggable_id}");
                let dug = dug.unwrap_or_else(|| {
                    panic!("{name} ({shovel_id}) left {diggable_id}'s tile empty")
                });
                assert_eq!(
                    dug.item_id,
                    diggable_id + 1,
                    "{name} ({shovel_id}) dug {diggable_id} into the wrong item"
                );
                assert_eq!(dug.config.name, "hole");
            }
        }
    }

    #[test]
    fn every_rope_hauls_the_player_up_a_rope_spot() {
        let config = &GAME_CONFIG.multi_action;
        let rope_spot = config.rope_spot_ids[0];

        for rope_id in &config.rope_ids {
            let (here, spot) = (Position::new(10, 10, 7), Position::new(10, 11, 7));
            let above = Position::new(10, 10, 6);
            let mut map = GameMap::new();

            let (rope, target) = (an_item(*rope_id), an_item(rope_spot));
            let (rope_guid, target_guid) = (rope.guid.clone(), target.guid.clone());
            map.insert_tile(here.clone(), a_tile_with(rope));
            map.insert_tile(spot.clone(), a_tile_with(target));
            // The only tile `firt_available_position_up` can find, so the destination is known.
            map.insert_tile(above.clone(), a_tile_with(an_item(rope_spot)));
            let agent = map
                .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &here)
                .unwrap();

            let (broadcasts, _) = use_item_with(
                &mut map,
                agent,
                ItemRef {
                    guid: rope_guid,
                    placement: ItemPlacement::Map(here),
                },
                ItemRef {
                    guid: target_guid,
                    placement: ItemPlacement::Map(spot),
                },
                0,
            );

            let name = &ITEM_CONFIGS[rope_id].name;
            assert!(
                !broadcasts
                    .iter()
                    .any(|b| matches!(b, BroadcastMessage::UseItemDenied { .. })),
                "{name} ({rope_id}) was refused: {broadcasts:?}"
            );
            assert_eq!(
                map.agent_position(agent),
                Some(&above),
                "{name} ({rope_id}) did not haul the player up"
            );
        }
    }

    #[test]
    fn a_non_tool_cannot_be_used_on_a_diggable() {
        let (here, sand_pos) = (Position::new(10, 10, 7), Position::new(10, 11, 7));
        let not_a_tool = 3459; // wooden hammer: usable and multiuse, but digs nothing.
        assert_eq!(GAME_CONFIG.multi_action.tool_action(not_a_tool), None);

        let mut map = GameMap::new();
        let hammer = an_item(not_a_tool);
        let sand = an_item(614);
        let (hammer_guid, sand_guid) = (hammer.guid.clone(), sand.guid.clone());
        map.insert_tile(here.clone(), a_tile_with(hammer));
        map.insert_tile(sand_pos.clone(), a_tile_with(sand));
        let agent = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &here)
            .unwrap();

        let (broadcasts, _) = use_item_with(
            &mut map,
            agent,
            ItemRef {
                guid: hammer_guid,
                placement: ItemPlacement::Map(here),
            },
            ItemRef {
                guid: sand_guid.clone(),
                placement: ItemPlacement::Map(sand_pos.clone()),
            },
            0,
        );

        assert!(
            broadcasts
                .iter()
                .any(|b| matches!(b, BroadcastMessage::UseItemDenied { .. })),
        );
        assert!(map.get_item_by_id(&sand_pos, &sand_guid).is_some());
    }
}
