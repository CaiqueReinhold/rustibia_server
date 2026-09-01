use tracing::{error, warn};

use crate::{
    actors::world::ScheduledCommand,
    entities::{
        agent::AgentKey,
        items::{Bounds, ItemFlag, ItemMultiAction, ItemRef},
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
        random::Rolls,
    },
};

pub fn use_item_with(
    map: &mut GameMap,
    agent_key: AgentKey,
    source: ItemRef,
    target: ItemRef,
    current_tick: Tick,
    roll: &mut Rolls,
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
        return use_item_failed("Item is too far".to_owned());
    }

    if !source_item.config.has_flag(ItemFlag::Usable) {
        return use_item_failed("Can't use that".to_owned());
    }

    if find_item_in_placement(map, &target).is_none() {
        return use_item_failed("Item was not found".to_owned());
    };

    // Two sources, and they answer different questions. What a shovel does is a
    // property of the world, so it stays in `game_conf.yaml`'s id lists; what a
    // potion restores is a property of the item, so it rides in the catalogue.
    let action = GAME_CONFIG
        .multi_action
        .tool_action(source_item.item_id)
        .or_else(|| source_item.config.attr_multi_action());
    if let Some(action) = action {
        match route_multi_action(
            &action,
            map,
            agent_key,
            &source,
            &target,
            current_tick,
            roll,
        ) {
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

#[allow(clippy::too_many_arguments)]
fn route_multi_action(
    action: &ItemMultiAction,
    map: &mut GameMap,
    agent_key: AgentKey,
    source: &ItemRef,
    target: &ItemRef,
    current_tick: Tick,
    roll: &mut Rolls,
) -> Result<(Vec<BroadcastMessage>, Vec<ScheduledCommand>), ItemActionError> {
    let mut broadcasts = Vec::new();
    let mut commands = Vec::new();
    match action {
        ItemMultiAction::Shovel => {
            shovel(&mut broadcasts, &mut commands, map, target, current_tick)?
        }
        ItemMultiAction::Rope => rope(&mut broadcasts, map, agent_key, target)?,
        ItemMultiAction::Potion { health, mana } => potion(
            &mut broadcasts,
            map,
            agent_key,
            source,
            *health,
            *mana,
            roll,
        )?,
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

#[allow(clippy::too_many_arguments)]
fn potion(
    broadcasts: &mut Vec<BroadcastMessage>,
    map: &mut GameMap,
    agent_key: AgentKey,
    potion: &ItemRef,
    health: Option<Bounds>,
    mana: Option<Bounds>,
    roll: &mut Rolls,
) -> Result<(), ItemActionError> {
    let health_roll = health.map(|b| roll.uniform(b.min, b.max));
    let mana_roll = mana.map(|b| roll.uniform(b.min, b.max));

    if remove_item_at(broadcasts, map, potion, 1).is_err() {
        return Err(ItemActionError::ActionFailed);
    }

    if let Some(amount) = health_roll {
        match map.get_agent_mut(agent_key) {
            Some(agent) => {
                agent.restore_life(amount);
                broadcasts.push(BroadcastMessage::AgentLifeUpdated { agent_key });
            }
            None => error!("agent {agent_key:?} vanished mid-drink; life not restored"),
        }
    }
    if let Some(amount) = mana_roll {
        match map.get_player_mut(agent_key) {
            Some(player) => {
                player.mana.add(amount);
                broadcasts.push(BroadcastMessage::PlayerManaUpdated { agent_key });
            }
            None => error!("agent {agent_key:?} vanished mid-drink; mana not restored"),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{
        agent::{Agent, Pool},
        items::{Item, ItemAttribute, ItemConfig, ItemId},
        map::MapTile,
    };
    use crate::persistence::items::ITEM_CONFIGS;
    use crate::persistence::test_fixtures::a_test_snapshot;
    use std::collections::HashSet;
    use std::sync::Arc;

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
            &mut Rolls::new(1),
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
                &mut Rolls::new(1),
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
            &mut Rolls::new(1),
        );

        assert!(
            broadcasts
                .iter()
                .any(|b| matches!(b, BroadcastMessage::UseItemDenied { .. })),
        );
        assert!(map.get_item_by_id(&sand_pos, &sand_guid).is_some());
    }

    // ── Potions ───────────────────────────────────────────────────────────────

    /// Built here rather than looked up: these tests are about what `drink` does
    /// with the bounds it is handed, not about the amounts `items.yaml` happens to
    /// carry. Id 9999 is in no tool list in `game_conf.yaml` either, so reaching
    /// this at all proves the catalogue is a real second source for the lookup.
    fn a_potion(amount: u8, health: Option<Bounds>, mana: Option<Bounds>) -> Item {
        Item::new(
            Arc::new(ItemConfig::new(
                9999,
                "test potion".to_string(),
                None,
                None,
                HashSet::from([
                    ItemFlag::Usable,
                    ItemFlag::Multiuse,
                    ItemFlag::Cumulative,
                    ItemFlag::Take,
                ]),
                HashSet::from([ItemAttribute::MultiAction(ItemMultiAction::Potion {
                    health,
                    mana,
                })]),
            )),
            amount,
        )
    }

    fn a_plain_item() -> Item {
        Item::new(
            Arc::new(ItemConfig::new(
                9998,
                "something to click".to_string(),
                None,
                None,
                HashSet::new(),
                HashSet::new(),
            )),
            1,
        )
    }

    fn bounds(min: u32, max: u32) -> Option<Bounds> {
        Some(Bounds { min, max })
    }

    fn pool(current: u32, maximum: u32) -> Pool {
        Pool { current, maximum }
    }

    struct Drunk {
        denied: bool,
        life: Pool,
        mana: Pool,
        charges_left: Option<u8>,
        broadcasts: Vec<BroadcastMessage>,
    }

    impl Drunk {
        fn broadcast_kinds(&self) -> Vec<&'static str> {
            self.broadcasts
                .iter()
                .filter_map(|b| match b {
                    BroadcastMessage::AgentLifeUpdated { .. } => Some("life"),
                    BroadcastMessage::PlayerManaUpdated { .. } => Some("mana"),
                    BroadcastMessage::UseItemDenied { .. } => Some("denied"),
                    _ => None,
                })
                .collect()
        }
    }

    /// Puts the potion on the drinker's own tile and something else beside them to
    /// be the thing clicked, then drinks `times` times at tick 0.
    fn drink_potion(potion: Item, life: Pool, mana: Pool, times: u8) -> Drunk {
        let (here, there) = (Position::new(10, 10, 7), Position::new(10, 11, 7));
        let mut map = GameMap::new();

        let potion_guid = potion.guid.clone();
        let target = a_plain_item();
        let target_guid = target.guid.clone();
        map.insert_tile(here.clone(), a_tile_with(potion));
        map.insert_tile(there.clone(), a_tile_with(target));

        let mut snapshot = a_test_snapshot(1, 1);
        snapshot.life = life;
        snapshot.mana = mana;
        let agent = map
            .insert_agent(Agent::from_player(snapshot), &here)
            .unwrap();

        let mut roll = Rolls::new(1);
        let mut broadcasts = Vec::new();
        for _ in 0..times {
            let (msgs, _) = use_item_with(
                &mut map,
                agent,
                ItemRef {
                    guid: potion_guid.clone(),
                    placement: ItemPlacement::Map(here.clone()),
                },
                ItemRef {
                    guid: target_guid.clone(),
                    placement: ItemPlacement::Map(there.clone()),
                },
                0,
                &mut roll,
            );
            broadcasts.extend(msgs);
        }

        Drunk {
            denied: broadcasts
                .iter()
                .any(|b| matches!(b, BroadcastMessage::UseItemDenied { .. })),
            life: map.get_agent(agent).unwrap().life().clone(),
            mana: map.get_player(agent).unwrap().mana.clone(),
            charges_left: map.get_top_item(&here).map(|item| item.amount),
            broadcasts,
        }
    }

    #[test]
    fn a_health_potion_restores_life_within_its_bounds_and_spends_one_charge() {
        let drunk = drink_potion(
            a_potion(3, bounds(100, 200), None),
            pool(10, 1000),
            pool(50, 1000),
            1,
        );

        assert!(!drunk.denied);
        assert!(
            (110..=210).contains(&drunk.life.current),
            "life {} is outside 10 + 100..200",
            drunk.life.current
        );
        assert_eq!(drunk.mana.current, 50, "a health potion touched the mana");
        assert_eq!(drunk.charges_left, Some(2));
        assert_eq!(drunk.broadcast_kinds(), ["life"]);
    }

    #[test]
    fn a_mana_potion_restores_mana_and_leaves_life_alone() {
        let drunk = drink_potion(
            a_potion(3, None, bounds(75, 125)),
            pool(10, 1000),
            pool(50, 1000),
            1,
        );

        assert!(!drunk.denied);
        assert_eq!(drunk.life.current, 10, "a mana potion healed");
        assert!(
            (125..=175).contains(&drunk.mana.current),
            "mana {} is outside 50 + 75..125",
            drunk.mana.current
        );
        assert_eq!(drunk.charges_left, Some(2));
        assert_eq!(drunk.broadcast_kinds(), ["mana"]);
    }

    /// The whole reason the two pools live in one action: a spirit potion cannot be
    /// two actions on one item, so this is the shape that has to work.
    #[test]
    fn a_spirit_potion_restores_both_pools_from_one_charge() {
        let drunk = drink_potion(
            a_potion(3, bounds(420, 580), bounds(180, 220)),
            pool(100, 2000),
            pool(100, 2000),
            1,
        );

        assert!(!drunk.denied);
        assert!(
            (520..=680).contains(&drunk.life.current),
            "life {}",
            drunk.life.current
        );
        assert!(
            (280..=320).contains(&drunk.mana.current),
            "mana {}",
            drunk.mana.current
        );
        assert_eq!(drunk.charges_left, Some(2), "one charge, both pools");
        assert_eq!(drunk.broadcast_kinds(), ["life", "mana"]);
    }

    #[test]
    fn neither_pool_can_be_filled_past_its_maximum() {
        let drunk = drink_potion(
            a_potion(1, bounds(500, 500), bounds(500, 500)),
            pool(990, 1000),
            pool(995, 1000),
            1,
        );

        assert_eq!(drunk.life.current, 1000);
        assert_eq!(drunk.mana.current, 1000);
    }

    /// TFS spends the flask regardless, and so does this. A potion that refused at
    /// full life would be a nicer rule and a different game.
    #[test]
    fn drinking_at_full_life_still_spends_the_charge() {
        let drunk = drink_potion(
            a_potion(3, bounds(100, 200), None),
            pool(1000, 1000),
            pool(50, 1000),
            1,
        );

        assert!(!drunk.denied);
        assert_eq!(drunk.life.current, 1000);
        assert_eq!(drunk.charges_left, Some(2));
    }

    /// `use_item_cooldown_ticks` is what stops a hotkey emptying a whole stack in
    /// one tick, and it is applied by `use_item_with` only when the action succeeded.
    #[test]
    fn a_second_drink_in_the_same_tick_is_refused() {
        let drunk = drink_potion(
            a_potion(3, bounds(100, 200), None),
            pool(10, 1000),
            pool(50, 1000),
            2,
        );

        assert!(drunk.denied, "the second drink went through");
        assert_eq!(drunk.charges_left, Some(2), "two charges were spent");
        assert_eq!(drunk.broadcast_kinds(), ["life", "denied"]);
    }

    /// The last stone in the stack leaves no item behind, and the pools still move.
    #[test]
    fn the_last_charge_removes_the_potion_entirely() {
        let drunk = drink_potion(
            a_potion(1, bounds(100, 200), None),
            pool(10, 1000),
            pool(50, 1000),
            1,
        );

        assert!(!drunk.denied);
        assert_eq!(
            drunk.charges_left, None,
            "an empty stack was left on the tile"
        );
        assert!(drunk.life.current > 10);
    }
}
