use crate::{
    entities::{
        agent::{Agent, AgentId, AgentKey},
        items::{Item, ItemGuid},
        map::GameMap,
        player::InventorySlot,
        skills::SkillType,
    },
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

pub fn find_item_in_slot<'a>(
    agent: &'a Agent,
    slot: InventorySlot,
    guid: &'a ItemGuid,
) -> Option<&'a Item> {
    let player = agent.get_player()?;
    player.inventory.get(&slot)?.find_by_guid(guid)
}
