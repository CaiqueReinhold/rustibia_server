use crate::{
    entities::{
        agent::{Agent, AgentKey},
        player::Player,
        spells::{CastTarget, Spell, SpellAttack, SpellEffect, SpellId},
    },
    game::{
        Tick, TickCtx,
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
    let Some(player) = ctx.map.get_player(agent_key) else {
        return;
    };
    let Some(agent) = ctx.map.get_agent(agent_key) else {
        return;
    };

    if !has_spell_requirements(spell, player) {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: "You can't use this spell".to_owned(),
        });
        return;
    }

    if player.mana().available() < spell.mana {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: "Not enough mana".to_owned(),
        });
        return;
    }

    if !can_cast_spell(agent, spell, ctx.tick) {
        ctx.events.push(BroadcastMessage::SpellDenied {
            agent_key,
            position,
            reason: "You're exausted".to_owned(),
        });
        return;
    }

    route_spell(ctx, agent_key, spell, target);
}

fn has_spell_requirements(spell: &Spell, player: &Player) -> bool {
    spell.level <= player.level() && spell.vocations.contains(&player.vocation())
}

fn can_cast_spell(agent: &Agent, spell: &Spell, current_tick: Tick) -> bool {
    agent.next_spell_group_tick(spell.group) >= current_tick
        && agent.next_spell_tick(spell.id) >= current_tick
}

fn route_spell(ctx: &mut TickCtx, agent_key: AgentKey, spell: &Spell, target: CastTarget) {
    for effect in &spell.effects {
        match effect {
            SpellEffect::Attack(attack) => attack_spell(ctx, agent_key, &target, attack, spell),
        }
    }
}

fn attack_spell(
    ctx: &mut TickCtx,
    agent_key: AgentKey,
    target: &CastTarget,
    spell_attack: &SpellAttack,
    spell: &Spell,
) {
    if let Some(plan) = plan_spell_attack(
        ctx.map,
        agent_key,
        ctx.roll,
        spell_attack,
        target,
        spell.mana,
    ) {
        execute_attack(ctx, plan);
    }
    if let Some(agent) = ctx.map.get_agent_mut(agent_key) {
        agent.stamp_spell(ctx.tick, spell);
        agent.stamp_auto_attack(ctx.tick);
    }
}
