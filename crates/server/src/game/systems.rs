use crate::{
    entities::agent::AgentKey,
    game::{TickCtx, combat, targeting},
};

pub fn combat_system(ctx: &mut TickCtx) {
    let with_targets: Vec<AgentKey> = ctx
        .map
        .iter_agents()
        .filter(|(_, agent)| agent.target().is_some())
        .map(|(key, _)| key)
        .collect();

    for agent_key in with_targets {
        if targeting::drop_unreachable_target(ctx, agent_key) {
            continue;
        }
        drive_auto_attack(ctx, agent_key);
    }
}

fn drive_auto_attack(ctx: &mut TickCtx, agent_key: AgentKey) {
    let Some(plan) = combat::plan_auto_attack(ctx.map, agent_key, ctx.roll, ctx.tick) else {
        return;
    };
    if let Some(attacker) = ctx.map.get_agent_mut(plan.attacker) {
        attacker.stamp_auto_attack(ctx.tick);
    }
    combat::execute_attack(ctx, plan);
}
