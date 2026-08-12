use crate::entities::agent::AgentKey;
use crate::entities::map::GameMap;
use crate::game::events::BroadcastMessage;

/// Local speech. Read-only against the map — an utterance changes no state — but it runs
/// inside the world loop so that speech is ordered against movement, and so that spell
/// dispatch and NPC hearing have one place to land later.
pub fn say(map: &GameMap, agent_key: AgentKey, message: String) -> Vec<BroadcastMessage> {
    if map.get_agent(agent_key).is_none() {
        return Vec::new();
    }
    vec![BroadcastMessage::AgentSaid { agent_key, message }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::Agent;
    use crate::entities::map::MapTile;
    use crate::entities::position::Position;
    use crate::persistence::test_fixtures::a_test_snapshot;

    #[test]
    fn a_live_agent_produces_one_event() {
        let pos = Position::new(100, 100, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let key = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &pos)
            .unwrap();

        let events = say(&map, key, "hello".to_owned());

        assert_eq!(events.len(), 1);
        match &events[0] {
            BroadcastMessage::AgentSaid { agent_key, message } => {
                assert_eq!(*agent_key, key);
                assert_eq!(message, "hello");
            }
            other => panic!("expected AgentSaid, got {other:?}"),
        }
    }

    /// A player can log out between sending and the tick that processes it.
    #[test]
    fn an_agent_no_longer_on_the_map_produces_nothing() {
        let events = say(&GameMap::new(), AgentKey::default(), "hello".to_owned());
        assert!(events.is_empty());
    }
}
