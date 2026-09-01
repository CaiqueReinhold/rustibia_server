use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use thiserror::Error;

use crate::entities::agent::{OutfitColors, OutfitId, Pool};
use crate::entities::creature::{BloodType, CreatureKind, CreatureKindId, LootEntry};
use crate::entities::items::ItemId;

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

fn default_amount() -> u32 {
    1
}

#[derive(Deserialize)]
struct RawLootEntry {
    item_id: u16,
    chance: u32,
    #[serde(default = "default_amount")]
    amount: u32,
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
    corpse: ItemId,
    #[serde(default)]
    loot: Vec<RawLootEntry>,
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
                corpse: raw.corpse,
                loot_table: raw
                    .loot
                    .into_iter()
                    .map(|loot| LootEntry {
                        item_id: loot.item_id,
                        chance: loot.chance,
                        amount: loot.amount,
                    })
                    .collect(),
            };
            (id, Arc::new(kind))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CONFIG;
    use crate::persistence::items::ITEM_CONFIGS;

    /// Loot tables are transcribed from TFS, whose item ids are a different space to the
    /// one `items.yaml` is generated in. A wrong id fails silently at runtime — the drop
    /// is logged and swallowed — so nothing but this test notices.
    #[test]
    fn every_shipped_loot_and_corpse_id_resolves_to_an_item() {
        let creatures = load_creatures(&CONFIG.creatures_file_path).unwrap();
        let unknown: Vec<ItemId> = creatures
            .values()
            .flat_map(|kind| {
                std::iter::once(kind.corpse).chain(kind.loot_table.iter().map(|l| l.item_id))
            })
            .filter(|id| !ITEM_CONFIGS.contains_key(id))
            .collect();

        assert!(
            unknown.is_empty(),
            "ids missing from items.yaml: {unknown:?}"
        );
    }
}
