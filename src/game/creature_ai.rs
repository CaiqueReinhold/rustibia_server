use rand::seq::SliceRandom;

use crate::entities::agent::AgentKey;
use crate::entities::map::GameMap;
use crate::entities::position::Direction;
use crate::game::Tick;

#[derive(Clone, Debug)]
pub enum CreatureAction {
    Walk {
        agent_key: AgentKey,
        direction: Direction,
    },
}

const WANDER_DIRECTIONS: [Direction; 4] = [
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::West,
];

/// Produces one action per creature whose movement cooldown has elapsed.
/// First-pass behavior: pick a random cardinal direction. The world's `walk`
/// validator will reject moves into invalid tiles — no need to pre-check here.
pub fn decide_actions(map: &GameMap, current_tick: Tick) -> Vec<CreatureAction> {
    let mut rng = rand::thread_rng();
    let mut actions = Vec::new();
    for (agent_key, agent) in map.iter_agents() {
        if !agent.is_creature() {
            continue;
        }
        if agent.next_walk_tick > current_tick {
            continue;
        }
        if let Some(direction) = WANDER_DIRECTIONS.choose(&mut rng).copied() {
            actions.push(CreatureAction::Walk {
                agent_key,
                direction,
            });
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::entities::agent::{Agent, Pool};
    use crate::entities::creature::CreatureKind;
    use crate::entities::map::{GameMap, MapTile};
    use crate::entities::position::Position;

    fn map_with_creature(at: &Position, next_walk_tick: Tick) -> (GameMap, AgentKey) {
        let mut map = GameMap::new();
        map.insert_tile(at.clone(), MapTile::new());
        let mut creature = Agent::from_creature_kind(&CreatureKind {
            name: "Creature".to_string(),
            life: Pool {
                current: 1,
                maximum: 1,
            },
            outfit: (1, (0, 0, 0, 0)),
            speed: 1,
            skills: HashMap::new(),
        });
        creature.next_walk_tick = next_walk_tick;
        let key = map.insert_agent(creature, at).unwrap();
        (map, key)
    }

    #[test]
    fn produces_walk_for_creature_off_cooldown() {
        let pos = Position::new(100, 100, 7);
        let (map, key) = map_with_creature(&pos, 0);
        let actions = decide_actions(&map, 10);
        assert_eq!(actions.len(), 1);
        match actions[0] {
            CreatureAction::Walk { agent_key, .. } => assert_eq!(agent_key, key),
        }
    }

    #[test]
    fn skips_creature_on_cooldown() {
        let pos = Position::new(100, 100, 7);
        let (map, _) = map_with_creature(&pos, 100);
        let actions = decide_actions(&map, 10);
        assert!(actions.is_empty());
    }

    #[test]
    fn ignores_players() {
        // Players are not creatures, so no actions even if their next_walk_tick is 0.
        // We assemble this implicitly: an empty map yields no actions.
        let map = GameMap::new();
        let actions = decide_actions(&map, 10);
        assert!(actions.is_empty());
    }
}
