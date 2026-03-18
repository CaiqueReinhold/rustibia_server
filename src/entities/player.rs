use std::collections::HashMap;

use crate::entities::items::Item;

use super::map::Position;

pub type PlayerId = u32;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InventorySlot {
    Backpack,
    Head,
    Chest,
    Legs,
    Feet,
    LeftHand,
    RightHand,
}

#[derive(Clone, Debug)]
pub struct Skill {
    pub value: u32,
    pub current_ticks: u64,
    pub max_ticks: u64,
}

#[derive(Clone, Debug)]
pub struct Pool {
    pub current: u32,
    pub maximum: u32,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub position: Position,
    pub origin: Position,
    pub inventory: HashMap<InventorySlot, Item>,
    pub level: u32,
    pub magic: Skill,
    pub meele: Skill,
    pub life: Pool,
    pub mana: Pool,
    pub experience: Pool,
    pub base_speed: u32,
}
