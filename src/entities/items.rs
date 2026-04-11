use std::{collections::HashSet, fmt::Display, sync::Arc};

use uuid::Uuid;

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
    Weight(u32),
    FloorChange(FloorChangeDirection),
    Inventory(InventorySlot),
    TileFriction(u32),
    Action(ItemAction),
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
    pub fn new(item_id: ItemId, config: Arc<ItemConfig>, amount: u8) -> Self {
        let content = if config.has_flag(ItemFlag::Container) {
            Some(Vec::new())
        } else {
            None
        };
        Item {
            config,
            guid: ItemGuid(Uuid::now_v7().to_string()),
            item_id,
            amount,
            content,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.config.name
    }

    pub fn is_full(&self) -> bool {
        self.config
            .get_attributes()
            .find_map(|attr| match attr {
                ItemAttribute::Capacity(cap) => Some(*cap as usize),
                _ => None,
            })
            .map(|cap| self.content.as_ref().map_or(0, |content| content.len()) >= cap)
            .unwrap_or(false)
    }

    pub fn container_capacity(&self) -> Option<u8> {
        self.config.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Capacity(c) => Some(*c),
            _ => None,
        })
    }

    pub fn available_capacity(&self) -> Option<usize> {
        let cap = self.container_capacity()? as usize;
        let used = self.content.as_ref().map(|c| c.len()).unwrap_or(0);
        Some(cap.saturating_sub(used))
    }

    pub fn get_slot(&self) -> Option<InventorySlot> {
        self.config.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Inventory(s) => Some(*s),
            _ => None,
        })
    }

    pub fn total_weight(&self) -> u32 {
        let own = self
            .config
            .get_attributes()
            .find_map(|attr| match attr {
                ItemAttribute::Weight(w) => Some(*w),
                _ => None,
            })
            .unwrap_or(0)
            * self.amount as u32;
        let inner = self
            .content
            .as_ref()
            .map_or(0, |items| items.iter().map(|i| i.total_weight()).sum());
        own + inner
    }

    pub fn find_by_guid(&self, guid: &ItemGuid) -> Option<&Item> {
        if self.guid == *guid {
            return Some(self);
        }
        self.content
            .as_ref()?
            .iter()
            .find_map(|i| i.find_by_guid(guid))
    }

    pub fn find_by_guid_mut(&mut self, guid: &ItemGuid) -> Option<&mut Item> {
        if self.guid == *guid {
            return Some(self);
        }
        self.content
            .as_mut()?
            .iter_mut()
            .find_map(|i| i.find_by_guid_mut(guid))
    }

    pub fn get_action(&self) -> Option<ItemAction> {
        self.config.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Action(a) => Some(a.clone()),
            _ => None,
        })
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ItemAction {
    Transform { into: ItemId },
}
