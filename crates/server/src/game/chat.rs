use crate::entities::agent::AgentKey;
use crate::entities::map::GameMap;
use crate::game::admin::parse_command;
use crate::game::events::BroadcastMessage;

pub fn say(map: &mut GameMap, agent_key: AgentKey, message: String) -> Vec<BroadcastMessage> {
    if map.get_agent(agent_key).is_none() {
        return Vec::new();
    }
    let mut msgs = Vec::new();

    if parse_command(&message, map, agent_key, &mut msgs) {
        return msgs;
    }

    msgs.push(BroadcastMessage::AgentSaid { agent_key, message });
    msgs
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

        let events = say(&mut map, key, "hello".to_owned());

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
        let events = say(&mut GameMap::new(), AgentKey::default(), "hello".to_owned());
        assert!(events.is_empty());
    }
}
