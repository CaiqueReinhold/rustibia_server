use tracing::error;

use crate::{
    entities::{
        agent::AgentKey,
        combat::AttackCost,
        effects::AreaEffect,
        healing::{HealPlan, Restore, RestoreType},
        map::GameMap,
        spells::{CastTarget, SpellHealing, SpellTargetMode},
    },
    game::{
        TickCtx,
        combat::{resolve_area, resolve_area_origin},
        config::GAME_CONFIG,
        damage::get_spell_base_damage,
        events::BroadcastMessage,
        item_movement::remove_item_at,
        map_query::can_throw,
        pathfinding::chebyshev,
        random::Rolls,
        spells::{SpellCastingDenyReason, consume_mana},
    },
};

pub fn plan_healing_spell(
    map: &GameMap,
    caster: AgentKey,
    roll: &mut Rolls,
    spell: &SpellHealing,
    cast_target: &CastTarget,
) -> Result<HealPlan, SpellCastingDenyReason> {
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

    let (targets, area_effect) = match &spell.target {
        SpellTargetMode::Caster => ([caster].to_vec(), None),
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

            if chebyshev(position, target_pos) > *range
                || !can_throw(map, position, target_pos, true)
            {
                return Err(SpellCastingDenyReason::OutOfReach);
            }

            ([target].to_vec(), None)
        }
        SpellTargetMode::Area { origin, shape } => {
            let origin = resolve_area_origin(map, origin, position, cast_target)
                .ok_or(SpellCastingDenyReason::InvalidTarget)?;
            let (agents, delta) = resolve_area(map, origin, shape.get_delta(agent.facing()));
            (
                agents,
                Some(AreaEffect {
                    effect_id: GAME_CONFIG.effect_ids.healing_spell,
                    origin: origin.clone(),
                    delta,
                }),
            )
        }
    };

    let heal_value = get_spell_base_damage(
        map.get_player(caster)
            .ok_or(SpellCastingDenyReason::InvalidState(
                caster,
                "non player caster",
            ))?,
        spell.base_power,
        spell.level_factor,
        spell.magic_factor,
        spell.spread,
        roll,
    );

    Ok(HealPlan {
        caster,
        cost: AttackCost::None,
        restores: targets
            .into_iter()
            .map(|target| {
                (
                    target,
                    Restore {
                        life: Some(heal_value),
                        mana: None,
                    },
                )
            })
            .collect(),
        area_effect,
    })
}

pub fn execute_healing(ctx: &mut TickCtx, plan: HealPlan) {
    match plan.cost {
        AttackCost::Item(item) => {
            if let Err(e) = remove_item_at(ctx, &item, 1) {
                error!("Failed to consume item({:?}) on attack: {}", item, e);
            };
        }
        AttackCost::Mana(mana_cost) => {
            if let Some(agent) = ctx.map.get_agent_mut(plan.caster) {
                consume_mana(plan.caster, agent, mana_cost, ctx.events);
            }
        }
        AttackCost::None => {}
    }

    if let Some(area) = plan.area_effect {
        ctx.events
            .push(BroadcastMessage::AreaEffectAppeared { area_effect: area });
    }

    for (agent_key, restore) in &plan.restores {
        restore_agent(ctx, *agent_key, restore);
    }

    if plan
        .restores
        .iter()
        .find(|(key, _)| *key == plan.caster)
        .is_none()
    {
        let Some(caster_pos) = ctx.map.agent_position(plan.caster) else {
            return;
        };
        ctx.events.push(BroadcastMessage::AreaEffectAppeared {
            area_effect: AreaEffect {
                effect_id: GAME_CONFIG.effect_ids.healing_spell,
                origin: caster_pos.clone(),
                delta: Vec::new(),
            },
        });
    }
}

fn restore_agent(ctx: &mut TickCtx, agent_key: AgentKey, restore: &Restore) {
    let Some(pos) = ctx.map.agent_position(agent_key).cloned() else {
        return;
    };
    let Some(agent) = ctx.map.get_agent_mut(agent_key) else {
        return;
    };

    let life = restore
        .life
        .map(|amount| amount.min(agent.life().available()))
        .filter(|amount| *amount > 0);
    let mana = restore
        .mana
        .map(|amount| amount.min(agent.mana().available()))
        .filter(|amount| *amount > 0);

    if let Some(life) = life {
        agent.restore_life(life);
        ctx.events.push(BroadcastMessage::AgentHealed {
            agent_key,
            position: pos.clone(),
            amount: life,
            restore_type: RestoreType::Life,
        });
    }
    if let Some(mana) = mana {
        agent.restore_mana(mana);
        ctx.events.push(BroadcastMessage::AgentHealed {
            agent_key,
            position: pos.clone(),
            amount: mana,
            restore_type: RestoreType::Mana,
        });
    }
}
