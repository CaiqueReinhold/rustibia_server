use serde::Deserialize;

use crate::entities::agent::{OutfitColors, OutfitId, Pool};

pub type CreatureKindId = String;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BloodType {
    Blood,
    Poison,
}

#[derive(Clone, Debug)]
pub struct CreatureKind {
    pub name: String,
    pub life: Pool,
    pub outfit: (OutfitId, OutfitColors),
    pub speed: u16,
    pub auto_attack_damage: (u32, u32),
    pub blood_type: BloodType,
}
