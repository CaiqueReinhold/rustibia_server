use serde::Deserialize;

use crate::entities::{
    agent::{OutfitColors, OutfitId, Pool},
    items::FluidType,
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
}
