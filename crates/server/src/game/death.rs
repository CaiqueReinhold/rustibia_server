use tracing::info;

use crate::entities::agent::AgentKey;
use crate::entities::map::GameMap;
use crate::game::events::BroadcastMessage;
use crate::game::targeting;

pub fn reap(
    map: &mut GameMap,
    agent: AgentKey,
    source: Option<AgentKey>,
    msgs: &mut Vec<BroadcastMessage>,
) {
    let Some(dead) = map.get_agent(agent) else {
        return;
    };
    if !dead.is_creature() || dead.life().current > 0 {
        return;
    }

    let victim = dead.name();
    match source.and_then(|key| map.get_agent(key)).map(|a| a.name()) {
        Some(killer) => info!("{killer} killed {victim}"),
        None => info!("{victim} died"),
    }

    let still_targeting: Vec<AgentKey> = map
        .iter_agents()
        .filter(|(_, other)| other.target() == Some(agent))
        .map(|(key, _)| key)
        .collect();
    for key in still_targeting {
        msgs.extend(targeting::clear_target_if_current(map, key, agent));
    }

    if let Some((_, position)) = map.remove_agent(agent) {
        msgs.push(BroadcastMessage::AgentDespawned {
            agent_key: agent,
            position,
            snapshot: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::Agent;
    use crate::entities::map::MapTile;
    use crate::entities::position::Position;
    use crate::persistence::test_fixtures::{a_test_creature, a_test_snapshot};

    #[test]
    fn removes_the_creature_and_announces_it() {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let rat = map
            .insert_agent(a_test_creature("Rat", 0, (1, 2)), &pos)
            .unwrap();
        let mut msgs = Vec::new();

        reap(&mut map, rat, None, &mut msgs);

        assert!(map.get_agent(rat).is_none());
        assert!(matches!(
            msgs.as_slice(),
            [BroadcastMessage::AgentDespawned { agent_key, .. }] if *agent_key == rat
        ));
    }

    #[test]
    fn is_a_no_op_on_a_living_creature() {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let rat = map
            .insert_agent(a_test_creature("Rat", 5, (1, 2)), &pos)
            .unwrap();
        let mut msgs = Vec::new();

        reap(&mut map, rat, None, &mut msgs);

        assert!(map.get_agent(rat).is_some());
        assert!(msgs.is_empty());
    }

    #[test]
    fn clears_a_target_that_named_the_dead() {
        let rat_pos = Position::new(10, 10, 7);
        let hunter_pos = Position::new(11, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(rat_pos.clone(), MapTile::new());
        map.insert_tile(hunter_pos.clone(), MapTile::new());
        let rat = map
            .insert_agent(a_test_creature("Rat", 0, (1, 2)), &rat_pos)
            .unwrap();
        let hunter = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &hunter_pos)
            .unwrap();
        map.get_agent_mut(hunter).unwrap().set_target(Some(rat));
        let mut msgs = Vec::new();

        reap(&mut map, rat, Some(hunter), &mut msgs);

        assert_eq!(map.get_agent(hunter).unwrap().target(), None);
        assert!(msgs.iter().any(|m| matches!(
            m,
            BroadcastMessage::TargetChanged { agent_key, target: None } if *agent_key == hunter
        )));
    }

    #[test]
    fn leaves_a_bystander_targeting_someone_else_alone() {
        let rat_pos = Position::new(10, 10, 7);
        let hunter_pos = Position::new(11, 10, 7);
        let bystander_pos = Position::new(12, 10, 7);
        let third_pos = Position::new(13, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(rat_pos.clone(), MapTile::new());
        map.insert_tile(hunter_pos.clone(), MapTile::new());
        map.insert_tile(bystander_pos.clone(), MapTile::new());
        map.insert_tile(third_pos.clone(), MapTile::new());
        let rat = map
            .insert_agent(a_test_creature("Rat", 0, (1, 2)), &rat_pos)
            .unwrap();
        let hunter = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &hunter_pos)
            .unwrap();
        let bystander = map
            .insert_agent(Agent::from_player(a_test_snapshot(2, 2)), &bystander_pos)
            .unwrap();
        let third = map
            .insert_agent(a_test_creature("Cave Rat", 0, (1, 2)), &third_pos)
            .unwrap();
        map.get_agent_mut(hunter).unwrap().set_target(Some(rat));
        map.get_agent_mut(bystander)
            .unwrap()
            .set_target(Some(third));
        let mut msgs = Vec::new();

        reap(&mut map, rat, Some(hunter), &mut msgs);

        assert_eq!(map.get_agent(hunter).unwrap().target(), None);
        assert_eq!(map.get_agent(bystander).unwrap().target(), Some(third));
        assert!(!msgs.iter().any(|m| matches!(
            m,
            BroadcastMessage::TargetChanged { agent_key, .. } if *agent_key == bystander
        )));
    }

    #[test]
    fn is_a_no_op_on_a_player() {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let player = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &pos)
            .unwrap();
        let mut msgs = Vec::new();

        reap(&mut map, player, None, &mut msgs);

        assert!(map.get_agent(player).is_some());
        assert!(msgs.is_empty());
    }
}
