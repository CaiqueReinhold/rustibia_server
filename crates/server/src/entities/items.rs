use std::{collections::HashSet, fmt::Display, sync::Arc};

use uuid::Uuid;

use crate::{
    entities::{
        combat::{AmmoType, CombatElement, WeaponType},
        player::InventorySlot,
        position::{ItemPlacement, Position},
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
    Multiuse,
    Avoid,
    AmmoContainer,
    LiquidPool,
}

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
    Defense(u16),
    ExtraDef(i16),
    Armor(u16),
    Speed(i16),
}

#[derive(Debug)]
pub struct ItemConfig {
    pub id: ItemId,
    pub name: String,
    pub description: Option<String>,
    pub article: Option<String>,
    flags: HashSet<ItemFlag>,
    attributes: HashSet<ItemAttribute>,
}

impl ItemConfig {
    pub fn new(
        id: ItemId,
        name: String,
        description: Option<String>,
        article: Option<String>,
        flags: HashSet<ItemFlag>,
        attributes: HashSet<ItemAttribute>,
    ) -> Self {
        ItemConfig {
            id,
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

    fn get_attributes(&self) -> impl Iterator<Item = &ItemAttribute> {
        self.attributes.iter()
    }

    pub fn attr_capacity(&self) -> Option<u8> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Capacity(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_weight(&self) -> Option<u32> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Weight(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_floor_change(&self) -> Option<FloorChangeDirection> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::FloorChange(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_tile_friction(&self) -> Option<u16> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::TileFriction(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_action(&self) -> Option<ItemAction> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Action(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_multi_action(&self) -> Option<ItemMultiAction> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::MultiAction(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_decay(&self) -> Option<(Tick, ItemId)> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Decay { decay_to, duration } => Some((*duration, *decay_to)),
            _ => None,
        })
    }

    pub fn attr_armor(&self) -> Option<u16> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Armor(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_extra_def(&self) -> Option<i16> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::ExtraDef(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_defense(&self) -> Option<u16> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Defense(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_weapon_type(&self) -> Option<WeaponType> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::WeaponType(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_weapon_attack(&self) -> Option<u16> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::WeaponAttack(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_weapon_element(&self) -> Option<CombatElement> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::WeaponElement(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_ammo_type(&self) -> Option<AmmoType> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::AmmoType(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_weapon_range(&self) -> Option<u8> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::WeaponRange(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_mana_cost(&self) -> Option<u32> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::ManaCost(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_missile_id(&self) -> Option<u16> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::MissileId(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_speed(&self) -> Option<i16> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Speed(a) => Some(*a),
            _ => None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub guid: ItemGuid,
    pub config: Arc<ItemConfig>,
    pub item_id: ItemId,
    pub amount: u8,
    pub fluid: Option<FluidType>,
    pub content: Option<Vec<Item>>,
}

impl Item {
    pub fn new(config: Arc<ItemConfig>, amount: u8) -> Self {
        let content = if config.has_flag(ItemFlag::Container) {
            Some(Vec::new())
        } else {
            None
        };
        let item_id = config.id;
        Item {
            config,
            guid: ItemGuid(Uuid::now_v7().to_string()),
            item_id,
            amount,
            fluid: None,
            content,
        }
    }

    pub fn new_fluid(config: Arc<ItemConfig>, fluid: FluidType) -> Self {
        let item_id = config.id;
        Item {
            config,
            guid: ItemGuid(Uuid::now_v7().to_string()),
            item_id,
            amount: 1,
            fluid: Some(fluid),
            content: None,
        }
    }

    pub fn wire_subtype(&self) -> u8 {
        self.fluid.map(|f| f as u8).unwrap_or(self.amount)
    }

    pub fn get_name(&self) -> &str {
        &self.config.name
    }

    pub fn container_capacity(&self) -> Option<u8> {
        self.config.attr_capacity()
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
}

#[derive(Debug, Clone)]
pub struct ItemRef {
    pub guid: ItemGuid,
    pub placement: ItemPlacement,
}

#[derive(Debug, Clone)]
pub struct ClientItemRef {
    pub position: Position,
    pub item_id: ItemId,
    pub stack_index: u8,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Copy)]
pub enum ItemAction {
    Transform { into: ItemId },
}

/// An inclusive range rolled at the moment of use. `min == max` is a fixed amount.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Bounds {
    pub min: u32,
    pub max: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ItemMultiAction {
    Shovel,
    Rope,
    Potion {
        health: Option<Bounds>,
        mana: Option<Bounds>,
        flask: Option<ItemId>,
    },
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
            2886,
            "pool".to_string(),
            None,
            None,
            HashSet::new(),
            HashSet::new(),
        ));

        let mut pool = Item::new(Arc::clone(&config), 1);
        pool.fluid = Some(FluidType::Blood);
        let stack = Item::new(config, 37);

        assert_eq!(pool.wire_subtype(), 5, "the fluid, not the amount");
        assert_eq!(stack.wire_subtype(), 37, "the amount, as before");
    }
}
