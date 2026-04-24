use crate::entities::{agent::Pool, inventory::Inventory};

use super::position::Position;

pub type PlayerId = u32;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Copy)]
pub enum InventorySlot {
    Head,
    Amulet,
    Chest,
    Backpack,
    LeftHand,
    RightHand,
    BothHands,
    Ring,
    Legs,
    Feet,
    Trinket,
}

impl InventorySlot {
    pub fn as_id(&self) -> u32 {
        match self {
            InventorySlot::BothHands => 0,
            InventorySlot::Head => 1,
            InventorySlot::Amulet => 2,
            InventorySlot::Backpack => 3,
            InventorySlot::Chest => 4,
            InventorySlot::RightHand => 5,
            InventorySlot::LeftHand => 6,
            InventorySlot::Legs => 7,
            InventorySlot::Feet => 8,
            InventorySlot::Ring => 9,
            InventorySlot::Trinket => 10,
        }
    }

    pub fn from_id(id: u16) -> Option<Self> {
        match id {
            0 => Some(InventorySlot::BothHands),
            1 => Some(InventorySlot::Head),
            2 => Some(InventorySlot::Amulet),
            3 => Some(InventorySlot::Backpack),
            4 => Some(InventorySlot::Chest),
            5 => Some(InventorySlot::RightHand),
            6 => Some(InventorySlot::LeftHand),
            7 => Some(InventorySlot::Legs),
            8 => Some(InventorySlot::Feet),
            9 => Some(InventorySlot::Ring),
            10 => Some(InventorySlot::Trinket),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: PlayerId,
    pub account_id: i32,
    pub position: Position,
    pub origin: Position,
    pub mana: Pool,
    pub capacity: Pool,
    pub inventory: Inventory,
}

impl Player {
    pub fn can_carry(&self, additional_weight: u32) -> bool {
        self.capacity.current + additional_weight <= self.capacity.maximum
    }
}
