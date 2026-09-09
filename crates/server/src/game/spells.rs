use thiserror::Error;

use crate::{
    entities::{
        agent::{Agent, AgentKey},
        map::GameMap,
        player::Player,
        position::Position,
        skills::SkillType,
        spells::{
            AreaOrigin, CastTarget, PowerCurve, Spell, SpellAttack, SpellEffect, SpellGroup,
            SpellHealing, SpellId, SpellTargetMode,
        },
    },
    game::{
        Tick, TickCtx,
        combat::{execute_attack, plan_spell_attack},
        events::BroadcastMessage,
        healing::{execute_healing, plan_healing_spell},
        map_query::{can_target, can_throw},
        random::Rolls,
        skills::tick_skill,
    },
    persistence::spells::SPELLS,
};

#[derive(Error, Debug, Clone)]
pub enum SpellCastingDenyReason {
    #[error("Invalid State: {0:?} {1}")]
    InvalidState(AgentKey, &'static str),
    #[error("Invalid spell")]
    IdNotFound,
    #[error("Not enough mana")]
    NoMana,
    #[error("You can't cast that spell")]
    RequirementFailed,
    #[error("No target")]
    InvalidTarget,
    #[error("Target out of reach")]
    OutOfReach,
    #[error("You're exausted")]
    StillInCooldown,
}

pub fn cast_spell(ctx: &mut TickCtx, agent_key: AgentKey, spell_id: SpellId, target: CastTarget) {
    let Some(position) = ctx.map.agent_position(agent_key).cloned() else {
        return;
    };
    let Some(player) = ctx.map.get_player(agent_key) else {
        return;
    };
    let Some(agent) = ctx.map.get_agent(agent_key) else {
        return;
    };

    let Some(spell) = SPELLS.get(&spell_id) else {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: SpellCastingDenyReason::IdNotFound,
        });
        return;
    };

    if !has_spell_requirements(spell, player) {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: SpellCastingDenyReason::RequirementFailed,
        });
        return;
    }

    if !agent.mana().can_afford(spell.mana) {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: SpellCastingDenyReason::NoMana,
        });
        return;
    }

    if !can_cast_spell(agent, spell, ctx.tick) {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: SpellCastingDenyReason::StillInCooldown,
        });
        return;
    }

    let mark = ctx.mark();
    if let Err(reason) = execute_effects(ctx, agent_key, spell, target) {
        ctx.rollback_to(mark);
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason,
        });
    } else {
        ctx.events.push(BroadcastMessage::SpellCast {
            agent_key,
            position,
            spell_id,
        });
    }
}

pub struct SpellTargets {
    /// Includes the caster when an area covers it; a planner that must not hit its own
    /// caster filters this itself.
    pub keys: Vec<AgentKey>,
    /// The single target's tile, or an area's centre. `None` for a cast on the caster.
    pub aim: Option<Position>,
    pub delta: Option<Vec<(i8, i8)>>,
}

pub fn resolve_spell_targets(
    map: &GameMap,
    caster: AgentKey,
    mode: &SpellTargetMode,
    cast_target: &CastTarget,
) -> Result<SpellTargets, SpellCastingDenyReason> {
    let agent = map
        .get_agent(caster)
        .ok_or(SpellCastingDenyReason::InvalidState(
            caster,
            "spell caster not found",
        ))?;
    let position = map
        .agent_position(caster)
        .ok_or(SpellCastingDenyReason::InvalidState(
            caster,
            "spell caster not found",
        ))?;

    match mode {
        SpellTargetMode::Caster => Ok(SpellTargets {
            keys: Vec::from([caster]),
            aim: None,
            delta: None,
        }),
        SpellTargetMode::Target { range } => {
            let target = agent
                .target()
                .ok_or(SpellCastingDenyReason::InvalidTarget)?;
            let target_pos =
                map.agent_position(target)
                    .ok_or(SpellCastingDenyReason::InvalidState(
                        target,
                        "spell target not found",
                    ))?;

            if position.distance(target_pos) > *range || !can_throw(map, position, target_pos, true)
            {
                return Err(SpellCastingDenyReason::OutOfReach);
            }

            Ok(SpellTargets {
                keys: Vec::from([target]),
                aim: Some(target_pos.clone()),
                delta: None,
            })
        }
        SpellTargetMode::Area { origin, shape } => {
            let origin = resolve_area_origin(map, origin, position, cast_target)
                .ok_or(SpellCastingDenyReason::InvalidTarget)?;
            let (keys, delta) = resolve_area(map, origin, shape.get_delta(agent.facing()));
            Ok(SpellTargets {
                keys,
                aim: Some(origin.clone()),
                delta: Some(delta),
            })
        }
    }
}

/// The curve's centre scaled by the caster's level and magic level, rolled across its spread.
pub fn roll_power(player: &Player, curve: &PowerCurve, roll: &mut Rolls) -> u32 {
    let center = curve.base_power
        * (1.0
            + (f32::from(player.level()) * curve.level_factor / 100.0)
            + (f32::from(player.skill(SkillType::Magic)) * curve.magic_factor / 100.0));
    let min = (center * (1.0 - curve.spread)).max(0.0).round() as u32;
    let max = (center * (1.0 + curve.spread)).round() as u32;
    roll.damage_roll(min, max)
}

fn resolve_area_origin<'a>(
    map: &'a GameMap,
    origin: &'a AreaOrigin,
    caster_pos: &'a Position,
    cast_target: &'a CastTarget,
) -> Option<&'a Position> {
    match origin {
        AreaOrigin::Caster => Some(caster_pos),
        AreaOrigin::Target => match cast_target {
            CastTarget::Agent(key) => map
                .agent_position(*key)
                .filter(|pos| can_target(caster_pos, pos) && can_throw(map, caster_pos, pos, true)),
            CastTarget::Position(pos) => Some(pos),
            CastTarget::None => None,
        },
    }
}

fn resolve_area(
    map: &GameMap,
    origin: &Position,
    shape: &[(i8, i8)],
) -> (Vec<AgentKey>, Vec<(i8, i8)>) {
    let affected_area: Vec<((i8, i8), Position)> = shape
        .iter()
        .flat_map(|(dx, dy)| {
            origin
                .checked_offset(*dx as i32, *dy as i32)
                .map(|pos| ((*dx, *dy), pos))
        })
        .filter(|(_, pos)| can_throw(map, origin, pos, true))
        .collect();

    (
        affected_area
            .iter()
            .flat_map(|(_, pos)| map.iter_agents_at(pos).ok())
            .flatten()
            .copied()
            .collect(),
        affected_area.into_iter().map(|(delta, _)| delta).collect(),
    )
}

pub fn consume_mana(
    agent_key: AgentKey,
    agent: &mut Agent,
    mana_cost: u32,
    events: &mut Vec<BroadcastMessage>,
) {
    agent.remove_mana(mana_cost);
    events.push(BroadcastMessage::PlayerManaUpdated { agent_key });
    let Some(player) = agent.get_player_mut() else {
        return;
    };
    tick_skill(
        player,
        agent_key,
        SkillType::Magic,
        mana_cost as u64,
        events,
    );
}

fn has_spell_requirements(spell: &Spell, player: &Player) -> bool {
    spell.level <= player.level() && spell.vocations.contains(&player.vocation())
}

fn can_cast_spell(agent: &Agent, spell: &Spell, current_tick: Tick) -> bool {
    agent.next_spell_group_tick(spell.group) <= current_tick
        && agent.next_spell_tick(spell.id) <= current_tick
}

fn execute_effects(
    ctx: &mut TickCtx,
    agent_key: AgentKey,
    spell: &Spell,
    target: CastTarget,
) -> Result<(), SpellCastingDenyReason> {
    for effect in &spell.effects {
        match effect {
            SpellEffect::Attack(attack) => attack_spell(ctx, agent_key, &target, attack),
            SpellEffect::Healing(healing) => healing_spell(ctx, agent_key, &target, healing),
        }?;
    }

    let agent = ctx
        .map
        .get_agent_mut(agent_key)
        .ok_or(SpellCastingDenyReason::InvalidState(
            agent_key,
            "missing after casting spell sucessfully",
        ))?;
    agent.stamp_spell(ctx.tick, spell);
    if matches!(spell.group, SpellGroup::Attack) {
        agent.stamp_auto_attack(ctx.tick);
    }
    consume_mana(agent_key, agent, spell.mana, ctx.events);

    Ok(())
}

fn attack_spell(
    ctx: &mut TickCtx,
    agent_key: AgentKey,
    target: &CastTarget,
    spell_attack: &SpellAttack,
) -> Result<(), SpellCastingDenyReason> {
    let plan = plan_spell_attack(ctx.map, agent_key, ctx.roll, spell_attack, target)?;
    execute_attack(ctx, plan);
    Ok(())
}

fn healing_spell(
    ctx: &mut TickCtx,
    agent_key: AgentKey,
    target: &CastTarget,
    spell_healing: &SpellHealing,
) -> Result<(), SpellCastingDenyReason> {
    let plan = plan_healing_spell(ctx.map, agent_key, ctx.roll, spell_healing, target)?;
    execute_healing(ctx, plan);
    Ok(())
}
