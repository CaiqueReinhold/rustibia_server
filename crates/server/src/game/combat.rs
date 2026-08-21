use std::{collections::HashMap, sync::Arc};

use tracing::info;

use crate::{
    actors::world::ScheduledCommand,
    entities::{
        agent::{Agent, AgentKey},
        combat::{CombatDamage, CombatElement, WeaponType},
        creature::{BloodType, CreatureKind},
        items::{Item, ItemAttribute, ItemConfig, ItemFlag, ItemGuid, ItemId, ItemRef},
        map::GameMap,
        player::{InventorySlot, Player},
        position::{ItemPlacement, Position},
        skills::SkillType,
    },
    game::{
        Tick,
        events::BroadcastMessage,
        game_config::{Color, GAME_CONFIG},
        item_action::check_decay,
        map_query::can_throw,
        random::Rolls,
        skills::tick_skill,
    },
};

fn get_skill(player: &Player) -> (Option<SkillType>, u16) {
    match player.weapon_type() {
        WeaponType::None => (None, 100),
        WeaponType::Axe => (Some(SkillType::Axe), player.skill_axe()),
        WeaponType::Club => (Some(SkillType::Club), player.skill_club()),
        WeaponType::Sword => (Some(SkillType::Sword), player.skill_sword()),
        WeaponType::Bow | WeaponType::Crossbow | WeaponType::Distance => {
            (Some(SkillType::Distance), player.skill_distance())
        }
        // wand/rod damage is based on ML but a strike doesn't tick the skill
        WeaponType::Wand | WeaponType::Rod => (None, player.skill_magic()),
    }
}

fn get_max_damage(attack_value: u16, level: u16, skill_value: u16) -> u32 {
    (((level as f32) / 5.5) + (((skill_value as f32) / 3.5) * ((attack_value as f32) / 3.0)))
        .round() as u32
}

fn get_min_damange(attack_value: u16, level: u16, skill_value: u16) -> u32 {
    (((level as f32) / 5.0) + (((skill_value as f32) / 10.0) * ((attack_value as f32) / 10.0)))
        .round() as u32
}

fn get_player_base_damage(
    player: &Player,
    roll: &mut Rolls,
) -> (Option<SkillType>, CombatElement, u32) {
    let level = player.level();
    let (skill_type, skill_value) = get_skill(player);
    let min = get_min_damange(player.weapon_attack(), level, skill_value);
    let max = get_max_damage(player.weapon_attack(), level, skill_value);
    (
        skill_type,
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

pub fn auto_attack_target(
    map: &mut GameMap,
    agent_key: AgentKey,
    roll: &mut Rolls,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    current_tick: Tick,
) -> (Vec<BroadcastMessage>, Vec<ScheduledCommand>) {
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();

    let Some(attacker) = map.get_agent(agent_key) else {
        return (msgs, cmds);
    };
    let Some(attacker_pos) = map.agent_position(agent_key) else {
        return (msgs, cmds);
    };
    let Some(attacked_key) = attacker.target() else {
        return (msgs, cmds);
    };
    let Some(attacked_pos) = map.agent_position(attacked_key).cloned() else {
        return (msgs, cmds);
    };

    if attacker.next_attack_tick >= current_tick {
        return (msgs, cmds);
    }

    if !is_in_range(attacker, attacker_pos, &attacked_pos) {
        return (msgs, cmds);
    }

    if attacker.attack_range() > 1 && !can_throw(map, attacker_pos, &attacked_pos, true) {
        return (msgs, cmds);
    }

    let mut ammo_guid = None;
    if let Some(player) = attacker.get_player() {
        let ammo_item = player.weapon_ammo();
        ammo_guid = ammo_item.map(|it| it.guid.clone());
        let can_attack = match player.weapon_type() {
            WeaponType::Bow | WeaponType::Crossbow => ammo_guid.is_some(),
            WeaponType::Wand | WeaponType::Rod => player.has_enough_mana(player.weapon_mana_cost()),
            _ => true,
        };
        if !can_attack {
            return (msgs, cmds);
        }

        if let Some(item) = player.inventory.get(&InventorySlot::LeftHand) {
            let missile = item
                .config
                .get_attributes()
                .find_map(|attr| match attr {
                    ItemAttribute::MissileId(id) => Some(*id),
                    _ => None,
                })
                .or(ammo_item.and_then(|it| {
                    it.config.get_attributes().find_map(|attr| match attr {
                        ItemAttribute::MissileId(id) => Some(*id),
                        _ => None,
                    })
                }));
            if let Some(missile_id) = missile {
                msgs.push(BroadcastMessage::MissileLaunched {
                    from: attacker_pos.clone(),
                    to: attacked_pos.clone(),
                    sprite_id: missile_id,
                })
            }
        }
    }

    let (skill_type, element, base_dmg) = if attacker.is_creature() {
        let (element, damage) =
            get_creature_base_damage(attacker.get_creature_kind().unwrap(), roll);
        (None, element, damage)
    } else {
        get_player_base_damage(attacker.get_player().unwrap(), roll)
    };

    if let Some(attacker) = map.get_agent_mut(agent_key) {
        attacker.next_attack_tick = GAME_CONFIG.combat.auto_attack_ticks + current_tick;
    }

    // TODO: apply shield + armor + mitigation
    // TODO: apply element modifier

    if let Some(attacked) = map.get_agent_mut(attacked_key) {
        attacked.take_hit(base_dmg, Some(agent_key));
        info!("monster life: {:?}", attacked.life());
    }

    if let Some(player) = map.get_player_mut(agent_key) {
        match player.weapon_type() {
            WeaponType::Bow | WeaponType::Crossbow if let Some(ammo_guid) = ammo_guid => {
                consume_ammo(player, agent_key, ammo_guid, &mut msgs);
            }
            WeaponType::Distance => {}
            WeaponType::Rod | WeaponType::Wand => {
                consume_mana(agent_key, player, player.weapon_mana_cost(), &mut msgs);
            }
            _ => {}
        }

        if let Some(skill_type) = skill_type {
            tick_skill(player, agent_key, skill_type, 1, &mut msgs);
        }
    }

    let damage = CombatDamage {
        element,
        value: base_dmg,
        blocked_shield: false,
        blocked_armor: false,
    };
    msgs.push(BroadcastMessage::DamageTaken {
        agent_key: attacked_key,
        damage,
    });

    if matches!(element, CombatElement::Physical) {
        draw_blood(
            map,
            &mut msgs,
            &mut cmds,
            item_configs,
            &attacked_pos,
            attacked_key,
            current_tick,
        );
    }

    (msgs, cmds)
}

fn consume_ammo(
    player: &mut Player,
    agent_key: AgentKey,
    ammo_guid: ItemGuid,
    msgs: &mut Vec<BroadcastMessage>,
) {
    if let Some((_, Some((parent, _)))) =
        player
            .inventory
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

fn draw_blood(
    map: &mut GameMap,
    msgs: &mut Vec<BroadcastMessage>,
    cmds: &mut Vec<ScheduledCommand>,
    item_configs: &HashMap<ItemId, Arc<ItemConfig>>,
    attacked_pos: &Position,
    attacked_key: AgentKey,
    current_tick: Tick,
) {
    let Some(config) = item_configs.get(&GAME_CONFIG.combat.pool_item_id) else {
        return;
    };

    let mut guid = None;
    if let Ok(mut items) = map.iter_items(attacked_pos)
        && let Some(it) = items.find(|it| it.config.has_flag(ItemFlag::LiquidPool))
    {
        guid = Some(it.guid.clone());
    }

    if let Some(guid) = guid {
        map.remove_item_from_tile(attacked_pos, &guid, 1);
    }

    let Some(attacked) = map.get_agent(attacked_key) else {
        return;
    };
    let pool = Item::new_fluid(config.clone(), attacked.blood_type().get_fluid());
    if let Ok(item) = map.place_item(attacked_pos, None, None, pool) {
        check_decay(
            cmds,
            item,
            ItemPlacement::Map(attacked_pos.clone()),
            current_tick,
        );
    };
    msgs.push(BroadcastMessage::TileChanged {
        position: attacked_pos.clone(),
    })
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
