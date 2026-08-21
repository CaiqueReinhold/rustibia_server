use crate::{
    entities::{agent::AgentKey, player::Player, skills::SkillType},
    game::events::BroadcastMessage,
};

pub fn tick_skill(
    player: &mut Player,
    agent_key: AgentKey,
    skill: SkillType,
    ticks: u64,
    messages: &mut Vec<BroadcastMessage>,
) {
    let Some(skill_value) = player.skills.get_mut(&skill) else {
        return;
    };

    if skill_value.current_ticks + ticks >= skill_value.max_ticks {
        skill_value.current_ticks = skill_value.max_ticks - skill_value.current_ticks + ticks;
        skill_value.value += 1;
        messages.push(BroadcastMessage::SkillUpgraded {
            agent_key,
            skill_type: skill,
        });
    } else {
        skill_value.current_ticks += ticks;
        messages.push(BroadcastMessage::SkillProgressUpdated {
            agent_key,
            skill_type: skill,
        });
    }
}
