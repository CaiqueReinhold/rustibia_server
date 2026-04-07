use tracing::info;

use crate::{
    entities::{
        agent::{Agent, AgentKey},
        items::{Item, ItemGuid},
        map::GameMap,
        player::InventorySlot,
        skills::SkillType,
    },
    messages::ServerMessage,
};

pub fn get_player_desc(map: &GameMap, key: AgentKey) -> Option<ServerMessage> {
    info!("start");
    let agent = map.get_agent(key)?;
    info!("agent");
    let position = map.agent_position(key)?;
    info!("position");
    let exp = agent.get_skill(SkillType::Level)?;
    info!("exp");
    let player = agent.get_player()?;

    let slot_item = |slot: InventorySlot| player.inventory.get(&slot).map(|it| it.item_id);

    Some(ServerMessage::DescribePlayer {
        position: position.clone(),
        name: agent.name().to_string(),
        level: exp.value,
        life: agent.life().clone(),
        mana: player.mana.clone(),
        outfit: agent.outfit(),
        speed: agent.speed,
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
    let item = player.inventory.get(&slot)?;
    if item.guid == *guid {
        return Some(item);
    } else if let Some(content) = &item.content {
        return find_item_recursive(guid, content);
    }
    None
}

fn find_item_recursive<'a>(guid: &'a ItemGuid, content: &'a Vec<Item>) -> Option<&'a Item> {
    for item in content {
        if item.guid == *guid {
            return Some(item);
        }
    }

    for item in content {
        if let Some(content) = &item.content {
            let found = find_item_recursive(guid, content);
            if found.is_some() {
                return found;
            }
        }
    }
    None
}
