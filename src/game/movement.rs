use anyhow::Result;

use crate::entities::{
    agent::{AgentKey, Facing},
    map::GameMap,
    position::Direction,
};

use super::events::BroadcastMessage;

pub fn walk(
    map: &mut GameMap,
    current_tick: u64,
    direction: Direction,
    agent_key: AgentKey,
) -> Result<Vec<BroadcastMessage>> {
    let mut broadcasts = Vec::new();

    let agent = map
        .get_agent(agent_key)
        .ok_or_else(|| anyhow::anyhow!("agent {:?} not spawned", agent_key))?;
    let Some(current_pos) = map.agent_position(agent_key).cloned() else {
        return Err(anyhow::anyhow!("agent {:?} position not found", agent_key));
    };

    if agent.next_walk_tick > current_tick {
        broadcasts.push(BroadcastMessage::PlayerWalkDenied { agent_key });
        return Ok(broadcasts);
    }

    let new_pos = current_pos + direction;
    if !map.can_move(&new_pos, agent_key) {
        broadcasts.push(BroadcastMessage::PlayerWalkDenied { agent_key });
        return Ok(broadcasts);
    }

    let Some(tile_friction) = map.tile_friction(&new_pos) else {
        broadcasts.push(BroadcastMessage::PlayerWalkDenied { agent_key });
        return Ok(broadcasts);
    };

    let walk_ticks = map
        .get_agent(agent_key)
        .unwrap()
        .calculate_walk_ticks(tile_friction, direction.is_diagonal());
    map.get_agent_mut(agent_key).unwrap().next_walk_tick = current_tick + walk_ticks;

    map.move_agent(agent_key, &new_pos)?;
    broadcasts.push(BroadcastMessage::AgentMoved {
        agent_key,
        direction,
        to_position: new_pos,
    });

    Ok(broadcasts)
}

pub fn change_direction(
    map: &mut GameMap,
    agent_key: AgentKey,
    facing: Facing,
) -> Vec<BroadcastMessage> {
    let current_facing = map.get_agent(agent_key).map(|agent| agent.facing);
    if let Some(current_facing) = current_facing {
        if facing != current_facing {
            map.get_agent_mut(agent_key).unwrap().facing = facing;
            return vec![BroadcastMessage::AgentChangedDirection { agent_key, facing }];
        }
    }
    vec![]
}
