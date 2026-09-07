use crate::{
    entities::{
        agent::{Agent, AgentKey},
        map::GameMap,
        position::Position,
        spells::{CastTarget, Spell, SpellAttack, SpellEffect, SpellId},
    },
    game::{
        TickCtx,
        combat::{execute_attack, plan_spell_attack},
        events::BroadcastMessage,
    },
    persistence::spells::SPELLS,
};

pub fn cast_spell(ctx: &mut TickCtx, agent_key: AgentKey, spell_id: SpellId, target: CastTarget) {
    let Some(position) = ctx.map.agent_position(agent_key).cloned() else {
        return;
    };
    let Some(spell) = SPELLS.get(&spell_id) else {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: "Invalid spell".to_owned(),
        });
        return;
    };

    if !ctx
        .map
        .get_agent(agent_key)
        .is_some_and(|a| has_spell_requirements(spell, a))
    {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: "You can't use this spell".to_owned(),
        });
        return;
    }

    if !ctx
        .map
        .get_agent(agent_key)
        .is_some_and(|_| is_target_valid(ctx.map, &position, &spell, &target))
    {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: "Invalid target".to_owned(),
        });
    }

    route_spell(ctx, agent_key, &spell, target);
}

fn has_spell_requirements(spell: &Spell, agent: &Agent) -> bool {
    true
}

fn is_target_valid(map: &GameMap, position: &Position, spell: &Spell, target: &CastTarget) -> bool {
    true
}

fn route_spell(ctx: &mut TickCtx, agent_key: AgentKey, spell: &Spell, target: CastTarget) {
    for effect in &spell.effects {
        match effect {
            SpellEffect::Attack(attack) => attack_spell(ctx, agent_key, &target, attack),
        }
    }
}

fn attack_spell(
    ctx: &mut TickCtx,
    agent_key: AgentKey,
    target: &CastTarget,
    spell_attack: &SpellAttack,
) {
    if let Some(plan) =
        plan_spell_attack(ctx.map, agent_key, ctx.roll, ctx.tick, spell_attack, target)
    {
        execute_attack(ctx, plan);
    }
}
