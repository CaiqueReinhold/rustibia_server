use crate::entities::agent::AgentKey;
use crate::entities::map::GameMap;
use crate::game::events::BroadcastMessage;

/// Sets or clears `agent`'s attack target.
///
/// A `Some(target)` is rejected when the target is absent from the map or is the
/// actor itself. A rejection is **not** silent: it clears the target and says so,
/// so the client is never left drawing a square the server does not hold.
///
/// Viewport membership is deliberately not checked here. `SessionActor` only ever
/// names an agent it currently holds a local id for, and holding that id is what
/// "in my viewport" means. Re-deriving it here would be a second, weaker copy of
/// the same rule.
pub fn set_target(
    map: &mut GameMap,
    agent: AgentKey,
    target: Option<AgentKey>,
) -> Vec<BroadcastMessage> {
    if map.get_agent(agent).is_none() {
        return Vec::new();
    }

    let accepted = match target {
        Some(t) if t != agent && map.get_agent(t).is_some() => Some(t),
        _ => None,
    };

    let Some(actor) = map.get_agent_mut(agent) else {
        return Vec::new();
    };
    actor.set_target(accepted);

    vec![BroadcastMessage::TargetChanged {
        agent_key: agent,
        target: accepted,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::Agent;
    use crate::entities::map::{GameMap, MapTile};
    use crate::entities::position::Position;
    use crate::persistence::test_fixtures::a_test_snapshot;

    fn map_with_two_players() -> (GameMap, AgentKey, AgentKey) {
        let mut map = GameMap::new();
        let a = Position::new(10, 10, 7);
        let b = Position::new(11, 10, 7);
        map.insert_tile(a.clone(), MapTile::new());
        map.insert_tile(b.clone(), MapTile::new());
        let attacker = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &a)
            .unwrap();
        let victim = map
            .insert_agent(Agent::from_player(a_test_snapshot(2, 1)), &b)
            .unwrap();
        (map, attacker, victim)
    }

    fn target_of(map: &GameMap, key: AgentKey) -> Option<AgentKey> {
        map.get_agent(key).unwrap().target()
    }

    #[test]
    fn sets_a_valid_target_and_announces_it() {
        let (mut map, attacker, victim) = map_with_two_players();

        let msgs = set_target(&mut map, attacker, Some(victim));

        assert_eq!(target_of(&map, attacker), Some(victim));
        assert!(matches!(
            msgs.as_slice(),
            [BroadcastMessage::TargetChanged { agent_key, target: Some(t) }]
                if *agent_key == attacker && *t == victim
        ));
    }

    #[test]
    fn clears_on_none_and_announces_it() {
        let (mut map, attacker, victim) = map_with_two_players();
        set_target(&mut map, attacker, Some(victim));

        let msgs = set_target(&mut map, attacker, None);

        assert_eq!(target_of(&map, attacker), None);
        assert!(matches!(
            msgs.as_slice(),
            [BroadcastMessage::TargetChanged { agent_key, target: None }]
                if *agent_key == attacker
        ));
    }

    /// A rejection must still announce a clear. Returning nothing would leave the
    /// client drawing a square the server does not believe in.
    #[test]
    fn rejecting_a_missing_agent_announces_a_clear() {
        let (mut map, attacker, victim) = map_with_two_players();
        set_target(&mut map, attacker, Some(victim));
        map.remove_agent(victim);

        let msgs = set_target(&mut map, attacker, Some(victim));

        assert_eq!(target_of(&map, attacker), None);
        assert!(matches!(
            msgs.as_slice(),
            [BroadcastMessage::TargetChanged { target: None, .. }]
        ));
    }

    #[test]
    fn rejecting_self_target_announces_a_clear() {
        let (mut map, attacker, _victim) = map_with_two_players();

        let msgs = set_target(&mut map, attacker, Some(attacker));

        assert_eq!(target_of(&map, attacker), None);
        assert!(matches!(
            msgs.as_slice(),
            [BroadcastMessage::TargetChanged { target: None, .. }]
        ));
    }

    /// Switching targets is one event, not a clear followed by a set. A client
    /// that saw two would flicker.
    #[test]
    fn replacing_a_target_emits_exactly_one_event() {
        let (mut map, attacker, victim) = map_with_two_players();
        let third = Position::new(12, 10, 7);
        map.insert_tile(third.clone(), MapTile::new());
        let other = map
            .insert_agent(Agent::from_player(a_test_snapshot(3, 1)), &third)
            .unwrap();
        set_target(&mut map, attacker, Some(victim));

        let msgs = set_target(&mut map, attacker, Some(other));

        assert_eq!(msgs.len(), 1);
        assert_eq!(target_of(&map, attacker), Some(other));
    }

    /// An actor that has left the map produces nothing at all — there is no
    /// session left to tell.
    #[test]
    fn a_missing_actor_produces_no_events() {
        let (mut map, attacker, victim) = map_with_two_players();
        map.remove_agent(attacker);

        let msgs = set_target(&mut map, attacker, Some(victim));

        assert!(msgs.is_empty());
    }
}
