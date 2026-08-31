use tracing::error;

use crate::{
    entities::{
        agent::{AgentKey, Facing},
        items::{Item, ItemId},
        map::GameMap,
        position::Position,
    },
    game::events::BroadcastMessage,
    persistence::items::ITEM_CONFIGS,
};

pub fn parse_command(
    command: &str,
    map: &mut GameMap,
    agent_key: AgentKey,
    msgs: &mut Vec<BroadcastMessage>,
) -> bool {
    if !map.get_player(agent_key).map(|p| p.admin).unwrap_or(false) {
        return false;
    }

    let parts: Vec<&str> = command.split(" ").collect();
    match parts.first() {
        Some(&"/create") => {
            let Some(item_id): Option<u16> = parts.get(1).and_then(|p| p.parse().ok()) else {
                return true;
            };
            let amount: u8 = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(1);
            create_item(item_id, amount, agent_key, map, msgs);
            true
        }
        Some(cmd) => {
            error!("Invalid command: {}", cmd);
            false
        }
        None => false,
    }
}

pub fn create_item(
    id: ItemId,
    amount: u8,
    agent_key: AgentKey,
    map: &mut GameMap,
    msgs: &mut Vec<BroadcastMessage>,
) {
    let Some(config) = ITEM_CONFIGS.get(&id) else {
        return;
    };
    let item = Item::new(config.clone(), amount);
    let Some(facing) = map.get_agent(agent_key).map(|a| a.facing()) else {
        return;
    };
    let Some(pos) = map.agent_position(agent_key).cloned() else {
        return;
    };
    let at_pos = match facing {
        Facing::North => Position::new(pos.x, pos.y.saturating_sub(1), pos.z),
        Facing::East => Position::new(pos.x.saturating_add(1), pos.y, pos.z),
        Facing::South => Position::new(pos.x, pos.y.saturating_add(1), pos.z),
        Facing::West => Position::new(pos.x.saturating_sub(1), pos.y, pos.z),
    };
    let _ = map.place_item(&at_pos, None, None, item);
    msgs.push(BroadcastMessage::TileChanged { position: at_pos });
}
