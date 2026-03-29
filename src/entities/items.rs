use std::{collections::HashSet, fmt::Display, sync::Arc};

use crate::entities::player::InventorySlot;

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct ItemGuid(pub String);
pub type ItemId = u16;
pub type ContainerId = u16;

impl ItemGuid {
    pub fn new() -> Self {
        ItemGuid(uuid::Uuid::now_v7().to_string())
    }
}

impl Display for ItemGuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.0)
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ItemFlag {
    Ground,
    Unmove,
    Unpass,
    Take,
    FullBank,
    Bottom,
    Cumulative,
    Container,
    Usable,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum FloorChangeDirection {
    Up,
    Down,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ItemAttribute {
    Capacity(u8),
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

    pub fn has_flag(&self, flag: ItemFlag) -> bool {
        self.flags.contains(&flag)
    }

    pub fn get_attributes(&self) -> impl Iterator<Item = &ItemAttribute> {
        self.attributes.iter()
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub guid: ItemGuid,
    pub config: Arc<ItemConfig>,
    pub item_id: ItemId,
    pub amount: u8,
    pub content: Option<Vec<Item>>,
}

impl Item {
    pub fn get_name(&self) -> &str {
        &self.config.name
    }
}
