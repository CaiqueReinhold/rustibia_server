use std::{collections::HashMap, sync::Arc};

use crate::{
    actors::world::ScheduledCommand,
    entities::{
        agent::{Agent, AgentKey},
        combat::{CombatDamage, CombatElement, WeaponType},
        creature::{BloodType, CreatureKind},
        items::{Item, ItemAttribute, ItemConfig, ItemGuid, ItemId, ItemRef},
        map::GameMap,
        player::{InventorySlot, Player},
        position::{ItemPlacement, Position},
        skills::SkillType,
    },
    game::{
        Tick, damage,
        events::BroadcastMessage,
        game_config::{Color, GAME_CONFIG},
        map_query::can_throw,
        random::Rolls,
        skills::tick_skill,
    },
};

struct WeaponSkill {
    value: u16,
    trains: Option<SkillType>,
}

fn weapon_skill(player: &Player) -> WeaponSkill {
    let (trains, value) = match player.weapon_type() {
        WeaponType::None => (None, GAME_CONFIG.combat.unarmed_skill),
        WeaponType::Axe => (Some(SkillType::Axe), player.skill_axe()),
        WeaponType::Club => (Some(SkillType::Club), player.skill_club()),
        WeaponType::Sword => (Some(SkillType::Sword), player.skill_sword()),
        WeaponType::Bow | WeaponType::Crossbow | WeaponType::Distance => {
            (Some(SkillType::Distance), player.skill_distance())
        }
        WeaponType::Wand | WeaponType::Rod => (None, player.skill_magic()),
    };
    WeaponSkill { value, trains }
}

fn get_max_damage(attack_value: u16, level: u16, skill_value: u16) -> u32 {
    (((level as f32) / 5.5) + (((skill_value as f32) / 3.5) * ((attack_value as f32) / 3.0)))
        .round() as u32
}

fn get_min_damage(attack_value: u16, level: u16, skill_value: u16) -> u32 {
    (((level as f32) / 5.0) + (((skill_value as f32) / 10.0) * ((attack_value as f32) / 10.0)))
        .round() as u32
}

fn get_player_base_damage(
    player: &Player,
    roll: &mut Rolls,
) -> (Option<SkillType>, CombatElement, u32) {
    let level = player.level();
    let skill = weapon_skill(player);
    let min = get_min_damage(player.weapon_attack(), level, skill.value);
    let max = get_max_damage(player.weapon_attack(), level, skill.value);
    (
        skill.trains,
        player.weapon_element(),
        roll.damage_roll(min, max),
    )
}

fn get_creature_base_damage(creature: &CreatureKind, roll: &mut Rolls) -> (CombatElement, u32) {
    (
        CombatElement::Physical,
        roll.damage_roll(creature.auto_attack_damage.0, creature.auto_attack_damage.1),
    )
}

fn is_in_range(attacker: &Agent, attacker_pos: &Position, attacked_pos: &Position) -> bool {
    let r = attacker.attack_range();
    let dx = attacked_pos.x.abs_diff(attacker_pos.x);
    let dy = attacked_pos.y.abs_diff(attacker_pos.y);
    (r as u16) >= dx && (r as u16) >= dy && attacker_pos.z == attacked_pos.z
}

#[derive(Debug, PartialEq)]
pub enum AttackCost {
    None,
    Ammo(ItemGuid),
    Mana(u32),
}

#[derive(Debug)]
pub struct AttackPlan {
    pub attacker: AgentKey,
    pub target: AgentKey,
    pub from: Position,
    pub to: Position,
    pub damage: CombatDamage,
    pub cost: AttackCost,
    pub trains: Option<SkillType>,
    pub missile: Option<u16>,
}

fn missile_id(item: &Item) -> Option<u16> {
    item.config.get_attributes().find_map(|attr| match attr {
        ItemAttribute::MissileId(id) => Some(*id),
        _ => None,
    })
}

pub fn plan_auto_attack(
    map: &GameMap,
    attacker: AgentKey,
    roll: &mut Rolls,
    current_tick: Tick,
) -> Option<AttackPlan> {
    let agent = map.get_agent(attacker)?;
    let from = map.agent_position(attacker)?.clone();
    let target = agent.target()?;
    let to = map.agent_position(target)?.clone();

    if agent.next_attack_tick > current_tick {
        return None;
    }
    if !is_in_range(agent, &from, &to) {
        return None;
    }
    if agent.attack_range() > 1 && !can_throw(map, &from, &to, true) {
        return None;
    }

    let (cost, missile) = match agent.get_player() {
        Some(player) => {
            let ammo = player.weapon_ammo();
            let cost = match player.weapon_type() {
                WeaponType::Bow | WeaponType::Crossbow => AttackCost::Ammo(ammo?.guid.clone()),
                WeaponType::Wand | WeaponType::Rod => {
                    let mana_cost = player.weapon_mana_cost();
                    if !player.has_enough_mana(mana_cost) {
                        return None;
                    }
                    AttackCost::Mana(mana_cost)
                }
                _ => AttackCost::None,
            };
            let missile = player
                .weapon()
                .and_then(|weapon| missile_id(weapon).or_else(|| ammo.and_then(missile_id)));
            (cost, missile)
        }
        None => (AttackCost::None, None),
    };

    let (trains, element, value) = if agent.is_creature() {
        let (element, value) = get_creature_base_damage(agent.get_creature_kind()?, roll);
        (None, element, value)
    } else {
        get_player_base_damage(agent.get_player()?, roll)
    };

    Some(AttackPlan {
        attacker,
        target,
        from,
        to,
        damage: CombatDamage {
            element,
            value,
            blocked_shield: false,
            blocked_armor: false,
        },
        cost,
        trains,
        missile,
    })
}

pub fn execute_attack(
    map: &mut GameMap,
    plan: AttackPlan,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    current_tick: Tick,
    msgs: &mut Vec<BroadcastMessage>,
    cmds: &mut Vec<ScheduledCommand>,
) {
    if let Some(attacker) = map.get_agent_mut(plan.attacker) {
        attacker.next_attack_tick = GAME_CONFIG.combat.auto_attack_ticks + current_tick;
    }

    if let Some(sprite_id) = plan.missile {
        msgs.push(BroadcastMessage::MissileLaunched {
            from: plan.from,
            to: plan.to,
            sprite_id,
        });
    }

    if let Some(player) = map.get_player_mut(plan.attacker) {
        match plan.cost {
            AttackCost::Ammo(guid) => consume_ammo(player, plan.attacker, guid, msgs),
            AttackCost::Mana(mana_cost) => consume_mana(plan.attacker, player, mana_cost, msgs),
            AttackCost::None => {}
        }
        if let Some(skill_type) = plan.trains {
            tick_skill(player, plan.attacker, skill_type, 1, msgs);
        }
    }

    damage::apply_damage(
        map,
        plan.target,
        plan.damage,
        Some(plan.attacker),
        item_configs,
        current_tick,
        msgs,
        cmds,
    );
}

fn consume_ammo(
    player: &mut Player,
    agent_key: AgentKey,
    ammo_guid: ItemGuid,
    msgs: &mut Vec<BroadcastMessage>,
) {
    if let Some((_, Some((parent, _)))) =
        player
            .inventory_mut()
            .remove(InventorySlot::RightHand, &ammo_guid, 1)
    {
        msgs.push(BroadcastMessage::ContainerUpdated {
            item: ItemRef {
                guid: parent,
                placement: ItemPlacement::Inventory(InventorySlot::RightHand, agent_key),
            },
        });
    }
}

fn consume_mana(
    agent_key: AgentKey,
    player: &mut Player,
    mana_cost: u32,
    msgs: &mut Vec<BroadcastMessage>,
) {
    player.mana.remove(mana_cost);
    msgs.push(BroadcastMessage::PlayerManaUpdated { agent_key });
    tick_skill(player, agent_key, SkillType::Magic, mana_cost as u64, msgs);
}

pub fn get_damage_visuals(damage: &CombatDamage, attacked: &Agent) -> (u16, Color) {
    let blood_type = attacked.get_creature_kind().map(|c| &c.blood_type);
    let effect = match damage.element {
        CombatElement::Physical => match blood_type {
            Some(BloodType::Blood) => GAME_CONFIG.effect_ids.life_hit,
            Some(BloodType::Poison) => GAME_CONFIG.effect_ids.poison_hit,
            None => GAME_CONFIG.effect_ids.life_hit,
        },
        CombatElement::Ice => GAME_CONFIG.effect_ids.ice_hit,
        CombatElement::Earth => GAME_CONFIG.effect_ids.earth_hit,
        CombatElement::Energy => GAME_CONFIG.effect_ids.energy_hit,
        CombatElement::Fire => GAME_CONFIG.effect_ids.fire_hit,
    };
    let color = match damage.element {
        CombatElement::Physical => match blood_type {
            Some(BloodType::Blood) => GAME_CONFIG.text_colors.red,
            Some(BloodType::Poison) => GAME_CONFIG.text_colors.lightgreen,
            None => GAME_CONFIG.text_colors.red,
        },
        CombatElement::Ice => GAME_CONFIG.text_colors.skyblue,
        CombatElement::Earth => GAME_CONFIG.text_colors.lightgreen,
        CombatElement::Energy => GAME_CONFIG.text_colors.eletric_purple,
        CombatElement::Fire => GAME_CONFIG.text_colors.orange,
    };
    (effect, color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::Agent;
    use crate::entities::combat::AmmoType;
    use crate::entities::items::{ItemAttribute, ItemFlag};
    use crate::entities::map::MapTile;
    use crate::entities::skills::SkillValue;
    use crate::persistence::player::PlayerSnapshot;
    use crate::persistence::test_fixtures::{a_test_creature, a_test_snapshot};
    use std::collections::HashSet;

    fn a_config(
        id: ItemId,
        flags: HashSet<ItemFlag>,
        attrs: HashSet<ItemAttribute>,
    ) -> Arc<ItemConfig> {
        Arc::new(ItemConfig::new(
            id,
            "thing".to_string(),
            None,
            None,
            flags,
            attrs,
        ))
    }

    fn a_wand(mana_cost: u32) -> Item {
        Item::new(
            a_config(
                1,
                HashSet::new(),
                HashSet::from([
                    ItemAttribute::WeaponType(WeaponType::Wand),
                    ItemAttribute::ManaCost(mana_cost),
                    ItemAttribute::WeaponAttack(10),
                ]),
            ),
            1,
        )
    }

    fn a_bow(range: Option<u8>) -> Item {
        let mut attrs = HashSet::from([
            ItemAttribute::WeaponType(WeaponType::Bow),
            ItemAttribute::WeaponAttack(10),
        ]);
        if let Some(range) = range {
            attrs.insert(ItemAttribute::WeaponRange(range));
        }
        Item::new(a_config(2, HashSet::new(), attrs), 1)
    }

    fn a_bow_with_missile(missile: u16) -> Item {
        Item::new(
            a_config(
                7,
                HashSet::new(),
                HashSet::from([
                    ItemAttribute::WeaponType(WeaponType::Bow),
                    ItemAttribute::WeaponAttack(10),
                    ItemAttribute::MissileId(missile),
                ]),
            ),
            1,
        )
    }

    fn an_arrow(missile: Option<u16>) -> Item {
        let mut attrs = HashSet::from([ItemAttribute::AmmoType(AmmoType::Arrow)]);
        if let Some(missile) = missile {
            attrs.insert(ItemAttribute::MissileId(missile));
        }
        Item::new(a_config(3, HashSet::new(), attrs), 10)
    }

    fn a_quiver_holding(arrow: Item) -> Item {
        let mut quiver = Item::new(
            a_config(4, HashSet::from([ItemFlag::AmmoContainer]), HashSet::new()),
            1,
        );
        quiver.content = Some(vec![arrow]);
        quiver
    }

    fn a_quiver_of_arrows() -> Item {
        a_quiver_holding(an_arrow(None))
    }

    fn armed(left: Option<Item>, right: Option<Item>) -> PlayerSnapshot {
        let mut snapshot = a_test_snapshot(1, 1);
        if let Some(item) = left {
            snapshot.inventory.insert(InventorySlot::LeftHand, item);
        }
        if let Some(item) = right {
            snapshot.inventory.insert(InventorySlot::RightHand, item);
        }
        snapshot
    }

    fn duel(attacker: Agent, target: Agent) -> (GameMap, AgentKey, AgentKey) {
        let a = Position::new(10, 10, 7);
        let b = Position::new(11, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(a.clone(), MapTile::new());
        map.insert_tile(b.clone(), MapTile::new());
        let attacker = map.insert_agent(attacker, &a).unwrap();
        let target = map.insert_agent(target, &b).unwrap();
        map.get_agent_mut(attacker)
            .unwrap()
            .set_target(Some(target));
        (map, attacker, target)
    }

    #[test]
    fn an_unarmed_player_plans_a_free_attack() {
        let (map, attacker, target) = duel(
            Agent::from_player(a_test_snapshot(1, 1)),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        let plan = plan_auto_attack(&map, attacker, &mut roll, 0).unwrap();

        assert!(matches!(plan.cost, AttackCost::None));
        assert_eq!(plan.target, target);
    }

    #[test]
    fn a_wand_plans_a_mana_cost() {
        let (map, attacker, _) = duel(
            Agent::from_player(armed(Some(a_wand(20)), None)),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        let plan = plan_auto_attack(&map, attacker, &mut roll, 0).unwrap();

        assert!(matches!(plan.cost, AttackCost::Mana(20)));
    }

    #[test]
    fn a_bow_with_a_quiver_plans_an_ammo_cost() {
        let quiver = a_quiver_of_arrows();
        let arrow_guid = quiver.content.as_ref().unwrap()[0].guid.clone();
        let (map, attacker, _) = duel(
            Agent::from_player(armed(Some(a_bow(None)), Some(quiver))),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        let plan = plan_auto_attack(&map, attacker, &mut roll, 0).unwrap();

        assert_eq!(plan.cost, AttackCost::Ammo(arrow_guid));
    }

    #[test]
    fn a_bow_without_ammo_plans_nothing() {
        let (map, attacker, _) = duel(
            Agent::from_player(armed(Some(a_bow(None)), None)),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        assert!(plan_auto_attack(&map, attacker, &mut roll, 0).is_none());
    }

    #[test]
    fn a_wand_without_mana_plans_nothing() {
        let mut snapshot = armed(Some(a_wand(500)), None);
        snapshot.mana.current = 10;
        let (map, attacker, _) = duel(
            Agent::from_player(snapshot),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        assert!(plan_auto_attack(&map, attacker, &mut roll, 0).is_none());
    }

    #[test]
    fn an_unexpired_cooldown_plans_nothing() {
        let (mut map, attacker, _) = duel(
            Agent::from_player(a_test_snapshot(1, 1)),
            a_test_creature("Rat", 10, (1, 2)),
        );
        map.get_agent_mut(attacker).unwrap().next_attack_tick = 40;
        let mut roll = Rolls::new(1);

        assert!(plan_auto_attack(&map, attacker, &mut roll, 10).is_none());
    }

    #[test]
    fn a_target_out_of_range_plans_nothing() {
        let a = Position::new(10, 10, 7);
        let b = Position::new(20, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(a.clone(), MapTile::new());
        map.insert_tile(b.clone(), MapTile::new());
        let attacker = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &a)
            .unwrap();
        let target = map
            .insert_agent(a_test_creature("Rat", 10, (1, 2)), &b)
            .unwrap();
        map.get_agent_mut(attacker)
            .unwrap()
            .set_target(Some(target));
        let mut roll = Rolls::new(1);

        assert!(plan_auto_attack(&map, attacker, &mut roll, 0).is_none());
    }

    /// A ranged attacker three tiles away with an `Unpass` item on the line. A *missing*
    /// tile would not do: `GameMap::has_sight` ends in `.unwrap_or(true)`, so sight is
    /// clear across a hole in the map.
    #[test]
    fn a_blocked_line_of_sight_plans_nothing() {
        let a = Position::new(10, 10, 7);
        let b = Position::new(13, 10, 7);
        let wall = Position::new(11, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(a.clone(), MapTile::new());
        map.insert_tile(b.clone(), MapTile::new());
        map.insert_tile(wall.clone(), MapTile::new());
        map.place_item(
            &wall,
            None,
            None,
            Item::new(
                a_config(6, HashSet::from([ItemFlag::Unpass]), HashSet::new()),
                1,
            ),
        )
        .unwrap();
        let attacker = map
            .insert_agent(
                Agent::from_player(armed(Some(a_bow(Some(5))), Some(a_quiver_of_arrows()))),
                &a,
            )
            .unwrap();
        let target = map
            .insert_agent(a_test_creature("Rat", 10, (1, 2)), &b)
            .unwrap();
        map.get_agent_mut(attacker)
            .unwrap()
            .set_target(Some(target));
        let mut roll = Rolls::new(1);

        assert!(plan_auto_attack(&map, attacker, &mut roll, 0).is_none());
    }

    #[test]
    fn no_target_plans_nothing() {
        let (map, attacker, _) = duel(
            Agent::from_player(a_test_snapshot(1, 1)),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut map = map;
        map.get_agent_mut(attacker).unwrap().set_target(None);
        let mut roll = Rolls::new(1);

        assert!(plan_auto_attack(&map, attacker, &mut roll, 0).is_none());
    }

    #[test]
    fn a_target_absent_from_the_map_plans_nothing() {
        let (mut map, attacker, target) = duel(
            Agent::from_player(a_test_snapshot(1, 1)),
            a_test_creature("Rat", 10, (1, 2)),
        );
        map.remove_agent(target);
        let mut roll = Rolls::new(1);

        assert!(plan_auto_attack(&map, attacker, &mut roll, 0).is_none());
    }

    /// Pins the broadcast order the spec calls load-bearing: cost, then skill, then damage.
    /// The `Magic` skill entry is required — `tick_skill` returns without emitting for a skill
    /// the player does not have, and `a_test_snapshot` carries only `Level`.
    #[test]
    fn executing_sets_the_cooldown_and_spends_the_mana() {
        let mut snapshot = armed(Some(a_wand(20)), None);
        snapshot.skills.insert(
            SkillType::Magic,
            SkillValue {
                value: 20,
                current_ticks: 0,
            },
        );
        let (mut map, attacker, _) = duel(
            Agent::from_player(snapshot),
            a_test_creature("Rat", 100, (1, 2)),
        );
        let mut roll = Rolls::new(1);
        let plan = plan_auto_attack(&map, attacker, &mut roll, 7).unwrap();
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        execute_attack(&mut map, plan, &HashMap::new(), 7, &mut msgs, &mut cmds);

        let agent = map.get_agent(attacker).unwrap();
        assert_eq!(
            agent.next_attack_tick,
            GAME_CONFIG.combat.auto_attack_ticks + 7
        );
        assert_eq!(agent.get_player().unwrap().mana.current, 80);
        assert_eq!(broadcast_kinds(&msgs), ["mana", "skill", "damage"]);
    }

    /// `Distance` is its own weapon type but costs nothing to swing — it fell through the
    /// old consumption match's `=> {}` arm, and must fall through to `AttackCost::None` here.
    #[test]
    fn a_distance_weapon_plans_no_cost() {
        let spear = Item::new(
            a_config(
                5,
                HashSet::new(),
                HashSet::from([
                    ItemAttribute::WeaponType(WeaponType::Distance),
                    ItemAttribute::WeaponAttack(10),
                ]),
            ),
            1,
        );
        let (map, attacker, _) = duel(
            Agent::from_player(armed(Some(spear), None)),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        let plan = plan_auto_attack(&map, attacker, &mut roll, 0).unwrap();

        assert!(matches!(plan.cost, AttackCost::None));
    }

    #[test]
    fn executing_spends_one_arrow() {
        let (mut map, attacker, _) = duel(
            Agent::from_player(armed(Some(a_bow(None)), Some(a_quiver_of_arrows()))),
            a_test_creature("Rat", 100, (1, 2)),
        );
        let mut roll = Rolls::new(1);
        let plan = plan_auto_attack(&map, attacker, &mut roll, 0).unwrap();
        let (mut msgs, mut cmds) = (Vec::new(), Vec::new());

        execute_attack(&mut map, plan, &HashMap::new(), 0, &mut msgs, &mut cmds);

        let arrows = map
            .get_agent(attacker)
            .unwrap()
            .get_player()
            .unwrap()
            .inventory
            .get(&InventorySlot::RightHand)
            .unwrap()
            .content
            .as_ref()
            .unwrap()[0]
            .amount;
        assert_eq!(arrows, 9);
    }

    fn broadcast_kinds(msgs: &[BroadcastMessage]) -> Vec<&'static str> {
        msgs.iter()
            .map(|m| match m {
                BroadcastMessage::MissileLaunched { .. } => "missile",
                BroadcastMessage::PlayerManaUpdated { .. } => "mana",
                BroadcastMessage::SkillProgressUpdated { .. }
                | BroadcastMessage::SkillUpgraded { .. } => "skill",
                BroadcastMessage::ContainerUpdated { .. } => "ammo",
                BroadcastMessage::DamageTaken { .. } => "damage",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn a_weapon_carrying_a_missile_id_plans_that_missile() {
        let (map, attacker, _) = duel(
            Agent::from_player(armed(
                Some(a_bow_with_missile(37)),
                Some(a_quiver_of_arrows()),
            )),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        let plan = plan_auto_attack(&map, attacker, &mut roll, 0).unwrap();

        assert_eq!(plan.missile, Some(37));
    }

    #[test]
    fn a_missile_id_on_the_ammo_is_used_when_the_weapon_has_none() {
        let (map, attacker, _) = duel(
            Agent::from_player(armed(
                Some(a_bow(None)),
                Some(a_quiver_holding(an_arrow(Some(42)))),
            )),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        let plan = plan_auto_attack(&map, attacker, &mut roll, 0).unwrap();

        assert_eq!(plan.missile, Some(42));
    }

    #[test]
    fn an_unarmed_player_plans_no_missile() {
        let (map, attacker, _) = duel(
            Agent::from_player(a_test_snapshot(1, 1)),
            a_test_creature("Rat", 10, (1, 2)),
        );
        let mut roll = Rolls::new(1);

        let plan = plan_auto_attack(&map, attacker, &mut roll, 0).unwrap();

        assert_eq!(plan.missile, None);
    }
}
