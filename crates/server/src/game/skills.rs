use crate::{
    entities::{agent::AgentKey, map::GameMap, skills::SkillType},
    game::events::BroadcastMessage,
};

pub fn tick_skill(
    map: &mut GameMap,
    skill: SkillType,
    ticks: u64,
    agent_key: AgentKey,
    messages: &mut Vec<BroadcastMessage>,
) {
    let Some(player) = map.get_player_mut(agent_key) else {
        return;
    };
    let Some(skill_value) = player.skills.get_mut(&skill) else {
        return;
    };

    if skill_value.current_ticks + ticks >= skill_value.max_ticks {
        skill_value.current_ticks = skill_value.max_ticks - skill_value.current_ticks + ticks;
        skill_value.value += 1;
    } else {
        skill_value.current_ticks += ticks;
    }
}
