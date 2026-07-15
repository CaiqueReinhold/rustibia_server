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
    local_id::LocalIdMap,
    messages::ServerMessage,
};

pub fn get_player_desc(map: &GameMap, key: AgentKey, id: AgentId) -> Option<ServerMessage> {
    let agent = map.get_agent(key)?;
    let position = map.agent_position(key)?;
    let exp = agent.get_skill(SkillType::Level)?;
    let player = agent.get_player()?;

    let slot_item = |slot: InventorySlot| player.inventory.get(&slot).map(|it| it.item_id);

    Some(ServerMessage::DescribePlayer {
        agent_id: id,
        position: position.clone(),
        facing: agent.facing,
        name: agent.name().to_string(),
        level: exp.value,
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

pub fn get_agent_desc(agent: &Agent, agent_id: AgentId, position: Position) -> ServerMessage {
    ServerMessage::SpawnAgent {
        agent_id,
        outfit: agent.outfit(),
        position,
        facing: agent.facing,
        name: agent.name().to_owned(),
        life: agent.life().clone(),
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
