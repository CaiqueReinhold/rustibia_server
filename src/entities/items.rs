use std::{collections::HashSet, sync::Arc};

use crate::entities::player::InventorySlot;

pub type ItemId = u16;

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ItemFlag {
    Ground,
    Unmove,
    Unpass,
    Take,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum FloorChangeDirection {
    Up,
    Down,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ItemAttribute {
    Capacity(u32),
    Weigth(u32),
    FloorChange(FloorChangeDirection),
    Inventory(InventorySlot),
    TileFriction(u32),
}

#[derive(Debug)]
pub struct ItemConfig {
    name: String,
    description: Option<String>,
    article: Option<String>,
    flags: HashSet<ItemFlag>,
    attributes: HashSet<ItemAttribute>,
}

impl ItemConfig {
    pub fn new(
        name: String,
        description: Option<String>,
        article: Option<String>,
        flags: HashSet<ItemFlag>,
        attributes: HashSet<ItemAttribute>,
    ) -> Self {
        ItemConfig {
            name,
            description,
            article,
            flags,
            attributes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub config: Arc<ItemConfig>,
    pub guid: String,
    pub item_id: ItemId,
    pub amount: u8,
    pub content: Vec<Item>,
}
