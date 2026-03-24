use crate::{
    entities::{agent::AgentKey, map::GameMap},
    messages::ServerMessage,
};

pub fn get_player_desc(map: &GameMap, key: AgentKey) -> Option<ServerMessage> {
    let agent = map.get_agent(key)?;
    let player = agent.get_player()?;
    let position = map.agent_position(key)?;

    Some(ServerMessage::DescribePlayer {
        position: position.clone(),
        name: player.name.clone(),
        level: player.level,
        life: player.life.clone(),
        mana: player.mana.clone(),
        outfit: player.outfit,
    })
}
