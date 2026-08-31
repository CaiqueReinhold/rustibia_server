use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use thiserror::Error;

use crate::entities::agent::{OutfitColors, OutfitId, Pool};
use crate::entities::creature::{BloodType, CreatureKind, CreatureKindId};

#[derive(Error, Debug)]
pub enum CreaturesLoadError {
    #[error("I/O error: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    ParseError(#[from] serde_yaml::Error),
}

#[derive(Deserialize)]
struct RawOutfit {
    id: OutfitId,
    colors: OutfitColors,
}

#[derive(Deserialize)]
struct RawDamage {
    min: u32,
    max: u32,
}

#[derive(Deserialize)]
struct RawCreature {
    name: String,
    life: u32,
    outfit: RawOutfit,
    damage: RawDamage,
    speed: u16,
    blood_type: BloodType,
    armor: u16,
    defense: u16,
    experience: u32,
}

#[derive(Deserialize)]
struct CreaturesFile {
    creatures: HashMap<String, RawCreature>,
}

pub fn load_creatures(
    path: impl AsRef<Path>,
) -> Result<HashMap<CreatureKindId, Arc<CreatureKind>>, CreaturesLoadError> {
    let contents = fs::read_to_string(path)?;
    let file: CreaturesFile = serde_yaml::from_str(&contents)?;
    Ok(file
        .creatures
        .into_iter()
        .map(|(id, raw)| {
            let kind = CreatureKind {
                name: raw.name,
                life: Pool {
                    current: raw.life,
                    maximum: raw.life,
                },
                auto_attack_damage: (raw.damage.min, raw.damage.max),
                outfit: (raw.outfit.id, raw.outfit.colors),
                speed: raw.speed,
                blood_type: raw.blood_type,
                armor: raw.armor,
                defense: raw.defense,
                experience: raw.experience,
            };
            (id, Arc::new(kind))
        })
        .collect())
}
