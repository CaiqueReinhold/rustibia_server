use serde::Deserialize;

use crate::{
    entities::{
        agent::{OutfitColors, OutfitId, Pool},
        items::{FluidType, ItemId},
    },
    game::TickDelta,
};

pub type CreatureKindId = String;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BloodType {
    Blood,
    Poison,
}

impl BloodType {
    pub fn get_fluid(&self) -> FluidType {
        match self {
            BloodType::Blood => FluidType::Blood,
            BloodType::Poison => FluidType::Slime,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LootEntry {
    pub item_id: ItemId,
    pub chance: u32,
    pub amount: u32,
}

#[derive(Clone, Debug)]
pub struct CreatureVoices {
    pub cooldown: TickDelta,
    pub chance: u32,
    pub sentences: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CreatureKind {
    pub name: String,
    pub life: Pool,
    pub outfit: (OutfitId, OutfitColors),
    pub speed: u16,
    pub auto_attack_damage: (u32, u32),
    pub blood_type: BloodType,
    pub armor: u16,
    pub defense: u16,
    pub experience: u32,
    pub corpse: ItemId,
    pub loot_table: Vec<LootEntry>,
    pub flee_threshold: Option<u32>,
    pub say: CreatureVoices,
}
