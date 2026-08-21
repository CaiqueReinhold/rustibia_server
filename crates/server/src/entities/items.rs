use std::{collections::HashSet, fmt::Display, sync::Arc};

use uuid::Uuid;

use crate::{
    entities::{
        combat::{AmmoType, CombatElement, WeaponType},
        player::InventorySlot,
        position::ItemPlacement,
    },
    game::Tick,
};

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
    Avoid,
    AmmoContainer,
}

/// What a splash or fluid container holds.
///
/// The discriminants **are** the wire values, matching OTClient's `FluidsType`,
/// so nothing converts on the way out. TFS instead keeps an internal enum that
/// packs the colour into the low bits (`FLUID_BLOOD = FLUID_RED = 2`,
/// `FLUID_LAVA = FLUID_RED + 24`) and converts through a `fluidMap` on send;
/// that trick exists to avoid a lookup table in C, and there is no legacy
/// representation here that would justify inheriting it.
///
/// Mapping these to colours is a rendering concern and deliberately lives on
/// the client, not here.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum FluidType {
    None = 0,
    Water = 1,
    Mana = 2,
    Beer = 3,
    Oil = 4,
    Blood = 5,
    Slime = 6,
    Mud = 7,
    Lemonade = 8,
    Milk = 9,
    Wine = 10,
    Health = 11,
    Urine = 12,
    Rum = 13,
    FruitJuice = 14,
    CoconutMilk = 15,
    Tea = 16,
    Mead = 17,
    Ink = 18,
    Candy = 19,
    Chocolate = 20,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum FloorChangeDirection {
    Up,
    Down,
    North,
    East,
    South,
    West,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ItemAttribute {
    Capacity(u8),
    Weight(u32),
    FloorChange(FloorChangeDirection),
    Inventory(InventorySlot),
    TileFriction(u16),
    Action(ItemAction),
    MultiAction(ItemMultiAction),
    Decay { duration: Tick, decay_to: ItemId },
    WeaponType(WeaponType),
    WeaponAttack(u16),
    WeaponElement(CombatElement),
    AmmoType(AmmoType),
    WeaponRange(u8),
    ManaCost(u32),
    MissileId(u16),
}

#[derive(Debug)]
pub struct ItemConfig {
    pub name: String,
    pub description: Option<String>,
    pub article: Option<String>,
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
    /// `Some` only for splashes and fluid containers. Kept separate from
    /// `amount` so the overload exists on the wire and nowhere else.
    pub fluid: Option<FluidType>,
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
            fluid: None,
            content,
        }
    }

    /// The subtype byte this item puts on the wire: a fluid for a splash or a
    /// fluid container, a stack count for everything else. Same byte either way
    /// -- which is OT's format, and why this rule lives in one place rather than
    /// at each of the five sites that encode an item.
    pub fn wire_subtype(&self) -> u8 {
        self.fluid.map(|f| f as u8).unwrap_or(self.amount)
    }

    pub fn get_name(&self) -> &str {
        &self.config.name
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

    pub fn get_multi_action(&self) -> Option<ItemMultiAction> {
        self.config.get_attributes().find_map(|attr| match attr {
            ItemAttribute::MultiAction(a) => Some(a.clone()),
            _ => None,
        })
    }

    pub fn get_decay(&self) -> Option<(Tick, ItemId)> {
        self.config.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Decay { duration, decay_to } => Some((*duration, *decay_to)),
            _ => None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ItemRef {
    pub guid: ItemGuid,
    pub placement: ItemPlacement,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ItemAction {
    Transform { into: ItemId },
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ItemMultiAction {
    Shovel,
    Rope,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client repeats this enum with the same discriminants, and nothing
    /// links the two -- they are separate repositories. This literal is the pin:
    /// the matching assertion lives in `rustibia-client/src/items/fluid.rs`, and
    /// if the two ever disagree every fluid in the game silently recolours.
    ///
    /// One sample is enough only because the discriminants are written
    /// explicitly rather than left positional, so reordering the variants cannot
    /// change any value.
    #[test]
    fn blood_is_five_on_the_wire() {
        assert_eq!(FluidType::Blood as u8, 5);
    }

    /// The wire byte is overloaded the way OT overloads it, but the struct is
    /// not: a fluid never masquerades as a stack count internally, so nothing
    /// that reasons about quantities can read one by accident.
    #[test]
    fn a_fluid_item_sends_its_fluid_where_a_stack_sends_its_count() {
        let config = Arc::new(ItemConfig::new(
            "pool".to_string(),
            None,
            None,
            HashSet::new(),
            HashSet::new(),
        ));

        let mut pool = Item::new(2886, Arc::clone(&config), 1);
        pool.fluid = Some(FluidType::Blood);
        let stack = Item::new(2148, config, 37);

        assert_eq!(pool.wire_subtype(), 5, "the fluid, not the amount");
        assert_eq!(stack.wire_subtype(), 37, "the amount, as before");
    }
}
