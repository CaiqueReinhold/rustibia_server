use std::collections::HashMap;
use std::sync::Arc;

use crate::entities::vocation::Vocation;
use crate::entities::{
    agent::Pool,
    combat::{AmmoType, CombatElement, WeaponType},
    inventory::Inventory,
    items::{Item, ItemFlag},
    skills::{SkillType, SkillValue},
};

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
    pub name: String,
    pub account_id: i32,
    pub admin: bool,
    pub vocation: Vocation,
    pub position: Position,
    pub origin: Position,
    pub mana: Pool,
    pub capacity: u32,
    pub inventory: Arc<Inventory>,
    pub skills: HashMap<SkillType, SkillValue>,
    pub armor: u16,
    pub defense: u16,
}

impl Player {
    pub fn inventory_mut(&mut self) -> &mut Inventory {
        Arc::make_mut(&mut self.inventory)
    }

    pub fn capacity_available(&self) -> u32 {
        self.capacity.saturating_sub(self.inventory.carried_weight)
    }

    pub fn can_carry(&self, additional_weight: u32) -> bool {
        self.inventory.carried_weight + additional_weight <= self.capacity
    }

    pub fn has_enough_mana(&self, mana_cost: u32) -> bool {
        self.mana.current >= mana_cost
    }

    pub fn has_shield(&self) -> bool {
        self.inventory
            .get(&InventorySlot::RightHand)
            .map(|it| it.config.attr_defense().is_some())
            .unwrap_or(false)
    }

    pub fn weapon(&self) -> Option<&Item> {
        self.inventory.get(&InventorySlot::LeftHand)
    }

    pub fn weapon_element(&self) -> CombatElement {
        self.weapon_ammo()
            .and_then(|it| it.config.attr_weapon_element())
            .or_else(|| self.weapon().and_then(|it| it.config.attr_weapon_element()))
            .unwrap_or(CombatElement::Physical)
    }

    pub fn weapon_attack(&self) -> u16 {
        self.weapon_ammo()
            .and_then(|it| it.config.attr_weapon_attack())
            .or_else(|| self.weapon().and_then(|it| it.config.attr_weapon_attack()))
            .unwrap_or(5)
    }

    pub fn weapon_type(&self) -> WeaponType {
        self.weapon()
            .and_then(|it| it.config.attr_weapon_type())
            .unwrap_or(WeaponType::None)
    }

    pub fn weapon_ammo(&self) -> Option<&Item> {
        let weapon_type = self.weapon_type();
        if matches!(weapon_type, WeaponType::Crossbow | WeaponType::Bow) {
            self.inventory
                .get(&InventorySlot::RightHand)
                .filter(|it| it.config.has_flag(ItemFlag::AmmoContainer))
                .and_then(|quiv| {
                    quiv.content.as_ref().and_then(|content| {
                        content.iter().find(|it| {
                            it.config.attr_ammo_type().is_some_and(|at| {
                                matches!(
                                    (at, weapon_type),
                                    (AmmoType::Arrow, WeaponType::Bow)
                                        | (AmmoType::Bolt, WeaponType::Crossbow)
                                )
                            })
                        })
                    })
                })
        } else {
            None
        }
    }

    pub fn weapon_range(&self) -> u8 {
        self.weapon()
            .and_then(|it| it.config.attr_weapon_range())
            .unwrap_or(1)
    }

    pub fn weapon_mana_cost(&self) -> u32 {
        self.weapon()
            .and_then(|it| it.config.attr_mana_cost())
            .unwrap_or(0)
    }

    pub fn get_skill(&self, skill: SkillType) -> Option<&SkillValue> {
        self.skills.get(&skill)
    }

    pub fn skill_sword(&self) -> u16 {
        self.get_skill(SkillType::Sword)
            .map(|st| st.value)
            .unwrap_or(10)
    }

    pub fn skill_axe(&self) -> u16 {
        self.get_skill(SkillType::Axe)
            .map(|st| st.value)
            .unwrap_or(10)
    }

    pub fn skill_club(&self) -> u16 {
        self.get_skill(SkillType::Club)
            .map(|st| st.value)
            .unwrap_or(10)
    }

    pub fn skill_distance(&self) -> u16 {
        self.get_skill(SkillType::Distance)
            .map(|st| st.value)
            .unwrap_or(10)
    }

    pub fn skill_magic(&self) -> u16 {
        self.get_skill(SkillType::Magic)
            .map(|st| st.value)
            .unwrap_or(1)
    }

    pub fn skill_shielding(&self) -> u16 {
        self.get_skill(SkillType::Shielding)
            .map(|st| st.value)
            .unwrap_or(10)
    }

    pub fn level(&self) -> u16 {
        self.get_skill(SkillType::Level)
            .map(|st| st.value)
            .unwrap_or(1)
    }

    pub fn update_equipment_stats(&mut self) {
        self.update_armor();
        self.update_defense();
    }

    fn update_armor(&mut self) {
        self.armor = self
            .inventory
            .slots()
            .values()
            .filter_map(|it| it.config.attr_armor())
            .sum()
    }

    fn update_defense(&mut self) {
        let weapon_extra = self
            .inventory
            .get(&InventorySlot::LeftHand)
            .and_then(|w| w.config.attr_extra_def())
            .unwrap_or(0);
        self.defense = match self.inventory.get(&InventorySlot::RightHand) {
            Some(it) => (it.config.attr_defense().unwrap_or(0) as i16 + weapon_extra).max(0) as u16,
            None => self
                .inventory
                .get(&InventorySlot::LeftHand)
                .map(|it| {
                    (it.config.attr_defense().unwrap_or(0) as i16 + weapon_extra).max(0) as u16
                })
                .unwrap_or(0),
        }
    }
}
