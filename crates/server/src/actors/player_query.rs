use crate::{
    entities::{
        agent::{Agent, AgentId, AgentKey},
        items::{ContainerId, ItemGuid},
        map::GameMap,
        player::InventorySlot,
        position::{ItemPlacement, Position},
        skills::SkillType,
    },
    game::map_query::find_item_in_reach,
    game::skills::{progress_bp, total_experience},
    local_id::LocalIdMap,
    messages::{ServerMessage, SkillProgress},
};

pub fn get_player_desc(map: &GameMap, key: AgentKey, id: AgentId) -> Option<ServerMessage> {
    let agent = map.get_agent(key)?;
    let position = map.agent_position(key)?;
    let player = agent.get_player()?;

    let slot_item = |slot: InventorySlot| player.inventory.get(&slot).map(|it| it.item_id);

    Some(ServerMessage::DescribePlayer {
        agent_id: id,
        position: position.clone(),
        facing: agent.facing(),
        name: agent.name().to_string(),
        level: player.level(),
        life: agent.life().clone(),
        mana: player.mana.clone(),
        outfit: agent.outfit(),
        speed: agent.speed(),
        capacity: player.capacity.available(),
        inventory_head: slot_item(InventorySlot::Head),
        inventory_amulet: slot_item(InventorySlot::Amulet),
        inventory_backpack: slot_item(InventorySlot::Backpack),
        inventory_chest: slot_item(InventorySlot::Chest),
        inventory_right_hand: slot_item(InventorySlot::RightHand),
        inventory_left_hand: slot_item(InventorySlot::LeftHand),
        inventory_legs: slot_item(InventorySlot::Legs),
        inventory_feet: slot_item(InventorySlot::Feet),
        inventory_ring: slot_item(InventorySlot::Ring),
        inventory_trinket: slot_item(InventorySlot::Trinket),
    })
}

pub fn get_player_skills(map: &GameMap, key: AgentKey) -> Option<ServerMessage> {
    let player = map.get_player(key)?;

    let mut skills: Vec<(SkillType, SkillProgress)> = player
        .skills
        .iter()
        .map(|(skill, value)| {
            (
                skill.clone(),
                SkillProgress {
                    level: value.value,
                    percent_bp: progress_bp(player.vocation, skill, value),
                },
            )
        })
        .collect();
    skills.sort_by_key(|(skill, _)| skill.as_id());

    Some(ServerMessage::PlayerSkills {
        experience: player
            .skills
            .get(&SkillType::Level)
            .map(total_experience)
            .unwrap_or(0),
        skills,
    })
}

pub fn get_agent_desc(agent: &Agent, agent_id: AgentId, position: Position) -> ServerMessage {
    ServerMessage::SpawnAgent {
        agent_id,
        outfit: agent.outfit(),
        position,
        facing: agent.facing(),
        name: agent.name().to_owned(),
        life: agent.life().to_wire(),
        speed: agent.speed(),
    }
}

pub fn client_position_to_placement(
    position: Position,
    map: &GameMap,
    containers: &LocalIdMap<ItemGuid>,
    agent_key: AgentKey,
) -> Option<(ItemPlacement, Option<ItemGuid>)> {
    if position.is_container_coord() {
        let container_id = position.y as ContainerId;
        let guid = containers.get_global(container_id)?;
        let (item, placement) = find_item_in_reach(map, guid, agent_key)?;
        let guid = item
            .content
            .as_ref()
            .and_then(|content| content.get(position.z as usize))
            .map(|item| item.guid.clone());
        Some((placement, guid))
    } else if position.is_inventory_coord() {
        let slot = InventorySlot::from_id(position.y)?;
        Some((ItemPlacement::Inventory(slot, agent_key), None))
    } else {
        Some((ItemPlacement::Map(position), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::Agent;
    use crate::entities::map::MapTile;
    use crate::entities::skills::{SkillType, SkillValue};
    use crate::messages::ServerMessage;
    use crate::persistence::test_fixtures::a_test_snapshot;

    /// Rows arrive in id order regardless of how the player's `HashMap` iterates,
    /// so the frame is reproducible and a client can rely on it.
    #[tokio::test]
    async fn skills_are_sent_in_id_order_with_the_experience_total() {
        let position = Position::new(100, 100, 7);
        let mut map = GameMap::new();
        map.insert_tile(position.clone(), MapTile::new());
        let key = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &position)
            .unwrap();
        let player = map.get_player_mut(key).unwrap();
        player.skills.insert(
            SkillType::Sword,
            SkillValue {
                value: 11,
                current_ticks: 27,
            },
        );
        player.skills.insert(
            SkillType::Level,
            SkillValue {
                value: 8,
                current_ticks: 55,
            },
        );

        let message = get_player_skills(&map, key).unwrap();

        let ServerMessage::PlayerSkills { experience, skills } = message else {
            panic!("expected PlayerSkills");
        };
        assert_eq!(experience, 4255);
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].0, SkillType::Level);
        assert_eq!(skills[0].1.level, 8);
        assert_eq!(skills[1].0, SkillType::Sword);
        assert_eq!(skills[1].1.percent_bp, 4909);
    }

    /// A character whose `Level` row never loaded still gets a window, with a
    /// zero rather than a missing message.
    #[tokio::test]
    async fn a_missing_level_row_reports_no_experience() {
        let position = Position::new(100, 100, 7);
        let mut map = GameMap::new();
        map.insert_tile(position.clone(), MapTile::new());
        let key = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &position)
            .unwrap();
        map.get_player_mut(key).unwrap().skills.clear();

        let ServerMessage::PlayerSkills { experience, skills } =
            get_player_skills(&map, key).unwrap()
        else {
            panic!("expected PlayerSkills");
        };
        assert_eq!(experience, 0);
        assert!(skills.is_empty());
    }
}
