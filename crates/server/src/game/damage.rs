use crate::actors::world::ScheduledCommand;
use crate::entities::agent::AgentKey;
use crate::entities::combat::{CombatDamage, CombatElement};
use crate::entities::items::{Item, ItemFlag};
use crate::entities::map::GameMap;
use crate::entities::position::{ItemPlacement, Position};
use crate::game::events::BroadcastMessage;
use crate::game::game_config::GAME_CONFIG;
use crate::game::item_action::check_decay;
use crate::game::{Tick, death};
use crate::persistence::items::ITEM_CONFIGS;

/// The target may no longer be in the map when this returns: a lethal hit reaps it. Anything
/// a caller still needs to do to the target must happen before the call.
#[allow(clippy::too_many_arguments)]
pub fn apply_damage(
    map: &mut GameMap,
    target: AgentKey,
    mut damage: CombatDamage,
    source: Option<AgentKey>,
    current_tick: Tick,
    msgs: &mut Vec<BroadcastMessage>,
    cmds: &mut Vec<ScheduledCommand>,
) {
    let Some(agent) = map.get_agent(target) else {
        return;
    };
    let Some(target_pos) = map.agent_position(target).cloned() else {
        return;
    };

    // TODO: apply element modifier

    let life = agent.life().current;
    let survivable = if agent.is_creature() {
        life
    } else {
        life.saturating_sub(1) // TODO: player death
    };

    damage.value = damage.value.min(survivable);
    if damage.value == 0 {
        if damage.blocked_shield || damage.blocked_armor {
            msgs.push(BroadcastMessage::DamageTaken {
                agent_key: target,
                damage,
            });
        }
        return;
    }

    let element = damage.element;
    let applied = damage.value;

    if let Some(agent) = map.get_agent_mut(target) {
        agent.take_hit(applied);
    }

    msgs.push(BroadcastMessage::DamageTaken {
        agent_key: target,
        damage,
    });

    if matches!(element, CombatElement::Physical) {
        draw_blood(map, msgs, cmds, &target_pos, target, current_tick);
    }

    if map
        .get_agent(target)
        .is_some_and(|agent| agent.life().current == 0)
    {
        death::reap(map, target, source, msgs);
    }
}

fn draw_blood(
    map: &mut GameMap,
    msgs: &mut Vec<BroadcastMessage>,
    cmds: &mut Vec<ScheduledCommand>,
    attacked_pos: &Position,
    attacked_key: AgentKey,
    current_tick: Tick,
) {
    let Some(config) = ITEM_CONFIGS.get(&GAME_CONFIG.combat.pool_item_id) else {
        return;
    };
    let Some(attacked) = map.get_agent(attacked_key) else {
        return;
    };
    let fluid = attacked.blood_type().get_fluid();

    let (ground_depth, existing) = map
        .iter_items(attacked_pos)
        .map(|items| {
            let items: Vec<_> = items.collect();
            let depth = items
                .iter()
                .take_while(|it| it.config.has_flag(ItemFlag::Ground))
                .count();
            let existing = items
                .iter()
                .find(|it| it.config.has_flag(ItemFlag::LiquidPool))
                .map(|it| it.guid.clone());
            (depth, existing)
        })
        .unwrap_or((0, None));

    let pool = Item::new_fluid(config.clone(), fluid);
    let Ok(item) = map.place_item(attacked_pos, Some(ground_depth), None, pool) else {
        return;
    };
    check_decay(
        cmds,
        item,
        ItemPlacement::Map(attacked_pos.clone()),
        current_tick,
    );
    if let Some(guid) = existing {
        map.remove_item_from_tile(attacked_pos, &guid, 1);
    }
    msgs.push(BroadcastMessage::TileChanged {
        position: attacked_pos.clone(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::Agent;
    use crate::entities::map::MapTile;
    use crate::persistence::test_fixtures::{a_test_creature, a_test_snapshot};

    fn physical(value: u32) -> CombatDamage {
        CombatDamage {
            element: CombatElement::Physical,
            value,
            blocked_shield: false,
            blocked_armor: false,
        }
    }

    fn fire(value: u32) -> CombatDamage {
        CombatDamage {
            element: CombatElement::Fire,
            value,
            blocked_shield: false,
            blocked_armor: false,
        }
    }

    fn map_with_creature(life: u32) -> (GameMap, AgentKey, Position) {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let rat = map
            .insert_agent(a_test_creature("Rat", life, (1, 2)), &pos)
            .unwrap();
        (map, rat, pos)
    }

    fn map_with_player() -> (GameMap, AgentKey) {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let player = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &pos)
            .unwrap();
        (map, player)
    }

    fn reported_damage(msgs: &[BroadcastMessage]) -> Option<u32> {
        msgs.iter().find_map(|m| match m {
            BroadcastMessage::DamageTaken { damage, .. } => Some(damage.value),
            _ => None,
        })
    }

    #[test]
    fn overkill_reports_only_the_damage_that_landed() {
        let (mut map, rat, _) = map_with_creature(3);
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        apply_damage(&mut map, rat, physical(50), None, 0, &mut msgs, &mut cmds);

        assert_eq!(reported_damage(&msgs), Some(3));
    }

    #[test]
    fn a_lethal_hit_removes_the_creature() {
        let (mut map, rat, _) = map_with_creature(3);
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        apply_damage(&mut map, rat, physical(50), None, 0, &mut msgs, &mut cmds);

        assert!(map.get_agent(rat).is_none());
    }

    #[test]
    fn a_survivable_hit_leaves_the_creature_in_the_map() {
        let (mut map, rat, _) = map_with_creature(10);
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        apply_damage(&mut map, rat, physical(3), None, 0, &mut msgs, &mut cmds);

        assert_eq!(map.get_agent(rat).unwrap().life().current, 7);
        assert_eq!(reported_damage(&msgs), Some(3));
    }

    #[test]
    fn a_lethal_hit_on_a_player_leaves_one_life() {
        let (mut map, player) = map_with_player();
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        apply_damage(
            &mut map,
            player,
            physical(500),
            None,
            0,
            &mut msgs,
            &mut cmds,
        );

        assert_eq!(map.get_agent(player).unwrap().life().current, 1);
        assert_eq!(reported_damage(&msgs), Some(99));
    }

    #[test]
    fn a_player_at_one_life_takes_no_further_damage() {
        let (mut map, player) = map_with_player();
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());
        apply_damage(
            &mut map,
            player,
            physical(500),
            None,
            0,
            &mut msgs,
            &mut cmds,
        );
        msgs.clear();

        apply_damage(
            &mut map,
            player,
            physical(500),
            None,
            0,
            &mut msgs,
            &mut cmds,
        );

        assert_eq!(map.get_agent(player).unwrap().life().current, 1);
        assert!(msgs.is_empty());
    }

    #[test]
    fn a_missing_target_emits_nothing() {
        let (mut map, rat, _) = map_with_creature(10);
        map.remove_agent(rat);
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        apply_damage(&mut map, rat, physical(3), None, 0, &mut msgs, &mut cmds);

        assert!(msgs.is_empty());
    }

    #[test]
    fn physical_damage_splashes_blood() {
        let (mut map, rat, pos) = map_with_creature(10);
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        apply_damage(&mut map, rat, physical(3), None, 0, &mut msgs, &mut cmds);

        let pooled = map
            .iter_items(&pos)
            .unwrap()
            .any(|it| it.config.id == GAME_CONFIG.combat.pool_item_id);
        assert!(pooled);
    }

    #[test]
    fn a_non_physical_element_does_not_splash() {
        let (mut map, rat, pos) = map_with_creature(10);
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        apply_damage(&mut map, rat, fire(3), None, 0, &mut msgs, &mut cmds);

        let pooled = map
            .iter_items(&pos)
            .unwrap()
            .any(|it| it.config.id == GAME_CONFIG.combat.pool_item_id);
        assert!(!pooled);
    }
}
