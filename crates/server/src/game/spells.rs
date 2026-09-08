use thiserror::Error;

use crate::{
    entities::{
        agent::{Agent, AgentKey},
        player::Player,
        skills::SkillType,
        spells::{CastTarget, Spell, SpellAttack, SpellEffect, SpellGroup, SpellHealing, SpellId},
    },
    game::{
        Tick, TickCtx,
        combat::{execute_attack, plan_spell_attack},
        events::BroadcastMessage,
        healing::{execute_healing, plan_healing_spell},
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

    if agent.mana().current < spell.mana {
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
