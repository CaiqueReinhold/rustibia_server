use std::{collections::HashSet, fmt::Display, sync::Arc};

use strum::{EnumCount, EnumIter};
use uuid::Uuid;

use crate::{
    entities::{
        combat::{AmmoType, CombatElement, WeaponType},
        effects::MissileId,
        inventory::InventorySlot,
        position::{ItemPlacement, Position},
    },
    game::TickDelta,
    local_id::LocalId,
};

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct ItemGuid(pub String);
/// An item's identity in the catalogue loaded from `items.yaml`. Global and stable,
/// unlike the session-local ids a `LocalIdMap` mints.
#[derive(
    Copy, Clone, Eq, PartialEq, Hash, Debug, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct ItemId(pub u16);

impl Display for ItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
/// An open container as one player's session names it on the wire. Session-local and
/// reused — see `LocalIdMap`, which is the only thing that may mint one.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(transparent)]
pub struct ContainerId(pub u16);

impl LocalId for ContainerId {
    fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    fn raw(self) -> u16 {
        self.0
    }
}

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

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, EnumCount, EnumIter)]
#[repr(u8)]
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

impl ItemFlag {
    pub const fn bit(self) -> u32 {
        1 << self as u32
    }
}

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ItemFlags(u32);

impl ItemFlags {
    pub const fn new() -> Self {
        ItemFlags(0)
    }

    pub const fn with(self, flag: ItemFlag) -> Self {
        ItemFlags(self.0 | flag.bit())
    }

    pub const fn contains(self, flag: ItemFlag) -> bool {
        self.0 & flag.bit() != 0
    }
}

impl FromIterator<ItemFlag> for ItemFlags {
    fn from_iter<I: IntoIterator<Item = ItemFlag>>(iter: I) -> Self {
        iter.into_iter().fold(ItemFlags::new(), ItemFlags::with)
    }
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
    Decay {
        duration: TickDelta,
        decay_to: ItemId,
    },
    WeaponType(WeaponType),
    WeaponAttack(u16),
    WeaponElement(CombatElement),
    AmmoType(AmmoType),
    WeaponRange(u8),
    HitChance(i16),
    MaxHitChance(u8),
    ManaCost(u32),
    MissileId(MissileId),
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
    flags: ItemFlags,
    attributes: HashSet<ItemAttribute>,
}

impl ItemConfig {
    pub fn new(
        id: ItemId,
        name: String,
        description: Option<String>,
        article: Option<String>,
        flags: impl IntoIterator<Item = ItemFlag>,
        attributes: HashSet<ItemAttribute>,
    ) -> Self {
        ItemConfig {
            id,
            name,
            description,
            article,
            flags: flags.into_iter().collect(),
            attributes,
        }
    }

    pub fn has_flag(&self, flag: ItemFlag) -> bool {
        self.flags.contains(flag)
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

    pub fn attr_decay(&self) -> Option<(TickDelta, ItemId)> {
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

    pub fn attr_hit_chance(&self) -> Option<i16> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::HitChance(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_max_hit_chance(&self) -> Option<u8> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::MaxHitChance(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_mana_cost(&self) -> Option<u32> {
        self.get_attributes().find_map(|attr| match attr {
            ItemAttribute::ManaCost(a) => Some(*a),
            _ => None,
        })
    }

    pub fn attr_missile_id(&self) -> Option<MissileId> {
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
    use strum::IntoEnumIterator;

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

    /// Two variants sharing a bit is the one failure a bitfield has that a `HashSet`
    /// does not, and it would not look like a bug: an item would simply answer `true`
    /// to a flag nobody gave it. Set each flag alone and check every other one.
    #[test]
    fn every_flag_owns_a_bit_of_its_own() {
        for set in ItemFlag::iter() {
            let flags = ItemFlags::new().with(set);
            for other in ItemFlag::iter() {
                assert_eq!(
                    flags.contains(other),
                    other == set,
                    "{set:?} answers for {other:?}"
                );
            }
        }
    }

    /// `ItemConfig::new` folds whatever it is handed, so the same flag twice must not
    /// mean anything different from the flag once -- and the catalogue hands it a
    /// `Vec<String>` straight out of YAML, which can repeat.
    #[test]
    fn a_repeated_flag_is_the_same_as_one() {
        let config = ItemConfig::new(
            ItemId(1),
            "thing".to_string(),
            None,
            None,
            [ItemFlag::Take, ItemFlag::Container, ItemFlag::Take],
            HashSet::new(),
        );

        assert!(config.has_flag(ItemFlag::Take));
        assert!(config.has_flag(ItemFlag::Container));
        assert!(!config.has_flag(ItemFlag::Ground));
    }

    /// The wire byte is overloaded the way OT overloads it, but the struct is
    /// not: a fluid never masquerades as a stack count internally, so nothing
    /// that reasons about quantities can read one by accident.
    #[test]
    fn a_fluid_item_sends_its_fluid_where_a_stack_sends_its_count() {
        let config = Arc::new(ItemConfig::new(
            ItemId(2886),
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
