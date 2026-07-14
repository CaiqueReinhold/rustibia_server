use crate::entities::{
    agent::{AgentKey, Facing},
    items::FloorChangeDirection,
    map::GameMap,
    position::{Direction, Position},
};
use anyhow::Result;

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

    let new_pos = current_pos.clone() + direction;
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
    let floor_change = map.get_floor_change(&new_pos);

    broadcasts.push(BroadcastMessage::AgentMoved {
        agent_key,
        direction,
        from_position: current_pos,
        to_position: new_pos.clone(),
    });

    if let Some(floor_change) = floor_change {
        let position = match floor_change {
            FloorChangeDirection::Up => Position::new(new_pos.x, new_pos.y, new_pos.z - 1),
            FloorChangeDirection::Down => {
                if let Some(downstairs_change) =
                    map.get_floor_change(&Position::new(new_pos.x, new_pos.y, new_pos.z + 1))
                {
                    let (x, y) = match downstairs_change {
                        FloorChangeDirection::Up | FloorChangeDirection::Down => {
                            (new_pos.x, new_pos.y)
                        }
                        FloorChangeDirection::North => (new_pos.x, new_pos.y + 1),
                        FloorChangeDirection::East => (new_pos.x - 1, new_pos.y),
                        FloorChangeDirection::South => (new_pos.x, new_pos.y - 1),
                        FloorChangeDirection::West => (new_pos.x + 1, new_pos.y),
                    };
                    Position::new(x, y, new_pos.z + 1)
                } else {
                    Position::new(new_pos.x, new_pos.y, new_pos.z + 1)
                }
            }
            FloorChangeDirection::North => Position::new(new_pos.x, new_pos.y - 1, new_pos.z - 1),
            FloorChangeDirection::East => Position::new(new_pos.x + 1, new_pos.y, new_pos.z - 1),
            FloorChangeDirection::South => Position::new(new_pos.x, new_pos.y + 1, new_pos.z - 1),
            FloorChangeDirection::West => Position::new(new_pos.x - 1, new_pos.y, new_pos.z - 1),
        };
        map.move_agent(agent_key, &position)?;
        broadcasts.push(BroadcastMessage::AgentTeleport {
            agent_key,
            from_position: new_pos,
            to_position: position,
        });
    }

    Ok(broadcasts)
}

pub fn change_direction(
    map: &mut GameMap,
    agent_key: AgentKey,
    facing: Facing,
) -> Vec<BroadcastMessage> {
    let current_facing = map.get_agent(agent_key).map(|agent| agent.facing);
    if let Some(current_facing) = current_facing
        && facing != current_facing
    {
        map.get_agent_mut(agent_key).unwrap().facing = facing;
        let position = map.agent_position(agent_key).cloned().unwrap_or_default();
        return vec![BroadcastMessage::AgentChangedDirection {
            agent_key,
            facing,
            position,
        }];
    }
    vec![]
}
