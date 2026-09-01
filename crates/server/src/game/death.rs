use tracing::{error, info};

use crate::entities::agent::{Agent, AgentKey};
use crate::entities::items::Item;
use crate::entities::map::GameMap;
use crate::entities::position::ItemPlacement;
use crate::game::events::BroadcastMessage;
use crate::game::item_movement::insert_item_at;
use crate::game::random::Rolls;
use crate::game::{experience, targeting};
use crate::persistence::items::ITEM_CONFIGS;

pub fn reap(
    map: &mut GameMap,
    agent_key: AgentKey,
    source: Option<AgentKey>,
    msgs: &mut Vec<BroadcastMessage>,
    rolls: &mut Rolls,
) {
    let Some(dead) = map.get_agent(agent_key) else {
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

    experience::award(map, agent_key, msgs);

    let still_targeting: Vec<AgentKey> = map
        .iter_agents()
        .filter(|(_, other)| other.target() == Some(agent_key))
        .map(|(key, _)| key)
        .collect();
    for key in still_targeting {
        msgs.extend(targeting::lose_target(map, key));
    }

    let Some((agent, position)) = map.remove_agent(agent_key) else {
        return;
    };
    msgs.push(BroadcastMessage::AgentDespawned {
        agent_key,
        position: position.clone(),
        snapshot: None,
    });

    let agent_corpse = agent.get_corpse();
    let Some(config) = ITEM_CONFIGS.get(&agent_corpse) else {
        error!("creature corpse with id {} not found", agent_corpse);
        return;
    };
    let mut corpse = Item::new(config.clone(), 1);
    roll_creature_loot(&mut corpse, &agent, rolls);
    insert_item_at(msgs, map, corpse, None, &ItemPlacement::Map(position), None);
}

fn roll_creature_loot(corpse: &mut Item, agent: &Agent, rolls: &mut Rolls) {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::Agent;
    use crate::entities::map::MapTile;
    use crate::entities::position::Position;
    use crate::persistence::test_fixtures::{
        a_test_creature, a_test_creature_worth, a_test_snapshot,
    };

    #[test]
    fn removes_the_creature_and_announces_it() {
        let pos = Position::new(10, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let rat = map
            .insert_agent(a_test_creature("Rat", 0, (1, 2)), &pos)
            .unwrap();
        let mut msgs = Vec::new();
        let mut rolls = Rolls::new(1);

        reap(&mut map, rat, None, &mut msgs, &mut rolls);

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
        let mut rolls = Rolls::new(1);

        reap(&mut map, rat, None, &mut msgs, &mut rolls);

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
        map.get_agent_mut(hunter).unwrap().set_target(Some(rat), 0);
        let mut msgs = Vec::new();
        let mut rolls = Rolls::new(1);

        reap(&mut map, rat, Some(hunter), &mut msgs, &mut rolls);

        assert_eq!(map.get_agent(hunter).unwrap().target(), None);
        assert!(msgs.iter().any(|m| matches!(
            m,
            BroadcastMessage::AgentLostTarget { agent_key, .. } if *agent_key == hunter
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
        map.get_agent_mut(hunter).unwrap().set_target(Some(rat), 0);
        map.get_agent_mut(bystander)
            .unwrap()
            .set_target(Some(third), 0);
        let mut msgs = Vec::new();
        let mut rolls = Rolls::new(1);

        reap(&mut map, rat, Some(hunter), &mut msgs, &mut rolls);

        assert_eq!(map.get_agent(hunter).unwrap().target(), None);
        assert_eq!(map.get_agent(bystander).unwrap().target(), Some(third));
        assert!(!msgs.iter().any(|m| matches!(
            m,
            BroadcastMessage::AgentLostTarget { agent_key, .. } if *agent_key == bystander
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
        let mut rolls = Rolls::new(1);

        reap(&mut map, player, None, &mut msgs, &mut rolls);

        assert!(map.get_agent(player).is_some());
        assert!(msgs.is_empty());
    }

    #[test]
    fn a_kill_awards_the_creatures_experience_before_removing_it() {
        let rat_pos = Position::new(10, 10, 7);
        let hunter_pos = Position::new(11, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(rat_pos.clone(), MapTile::new());
        map.insert_tile(hunter_pos.clone(), MapTile::new());
        let rat = map
            .insert_agent(a_test_creature_worth("Rat", 0, (1, 2), 100), &rat_pos)
            .unwrap();
        let hunter = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &hunter_pos)
            .unwrap();
        map.get_agent_mut(rat).unwrap().record_damage(hunter, 100);
        let mut msgs = Vec::new();
        let mut rolls = Rolls::new(1);

        reap(&mut map, rat, Some(hunter), &mut msgs, &mut rolls);

        assert!(map.get_agent(rat).is_none());
        assert_eq!(
            map.get_player(hunter)
                .unwrap()
                .skills
                .get(&crate::entities::skills::SkillType::Level)
                .unwrap()
                .value,
            2
        );
    }

    #[test]
    fn a_creature_nobody_damaged_awards_nothing() {
        let rat_pos = Position::new(10, 10, 7);
        let hunter_pos = Position::new(11, 10, 7);
        let mut map = GameMap::new();
        map.insert_tile(rat_pos.clone(), MapTile::new());
        map.insert_tile(hunter_pos.clone(), MapTile::new());
        let rat = map
            .insert_agent(a_test_creature_worth("Rat", 0, (1, 2), 100), &rat_pos)
            .unwrap();
        let hunter = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &hunter_pos)
            .unwrap();
        let mut msgs = Vec::new();
        let mut rolls = Rolls::new(1);

        reap(&mut map, rat, Some(hunter), &mut msgs, &mut rolls);

        assert_eq!(
            map.get_player(hunter)
                .unwrap()
                .skills
                .get(&crate::entities::skills::SkillType::Level)
                .unwrap()
                .value,
            1
        );
    }
}
