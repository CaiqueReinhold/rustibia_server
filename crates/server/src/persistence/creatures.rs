use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use thiserror::Error;

use crate::entities::agent::{OutfitColors, OutfitId, Pool};
use crate::entities::creature::{
    BloodType, CreatureKind, CreatureKindId, CreatureVoices, LootEntry,
};
use crate::entities::items::ItemId;
use crate::game::Tick;

#[derive(Error, Debug)]
pub enum CreaturesLoadError {
    #[error("I/O error reading {path}: {source}")]
    ReadError {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("YAML parse error in {path}: {source}")]
    ParseError {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error("{path} has no file stem to take a creature kind id from")]
    UnnamedFile { path: PathBuf },
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
struct RawCreatureVoices {
    cooldown: Tick,
    chance: u32,
    #[serde(default)]
    sentences: Vec<String>,
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
    #[serde(default)]
    flee_threshold: Option<u32>,
    say: RawCreatureVoices,
}

impl RawCreature {
    fn into_kind(self) -> CreatureKind {
        CreatureKind {
            name: self.name,
            life: Pool {
                current: self.life,
                maximum: self.life,
            },
            auto_attack_damage: (self.damage.min, self.damage.max),
            outfit: (self.outfit.id, self.outfit.colors),
            speed: self.speed,
            blood_type: self.blood_type,
            armor: self.armor,
            defense: self.defense,
            experience: self.experience,
            corpse: self.corpse,
            loot_table: self
                .loot
                .into_iter()
                .map(|loot| LootEntry {
                    item_id: loot.item_id,
                    chance: loot.chance,
                    amount: loot.amount,
                })
                .collect(),
            flee_threshold: self.flee_threshold,
            say: CreatureVoices {
                cooldown: self.say.cooldown,
                chance: self.say.chance,
                sentences: self.say.sentences,
            },
        }
    }
}

/// Each `.yaml` file in `dir` is one creature, and **its file stem is the
/// `CreatureKindId`** — the name `spawns.yaml` refers to. Renaming a file renames the
/// kind, which nothing but a spawn point's `kind:` will notice.
pub fn load_creatures(
    dir: impl AsRef<Path>,
) -> Result<HashMap<CreatureKindId, Arc<CreatureKind>>, CreaturesLoadError> {
    let dir = dir.as_ref();
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|source| CreaturesLoadError::ReadError {
            path: dir.to_path_buf(),
            source,
        })?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|source| CreaturesLoadError::ReadError {
                    path: dir.to_path_buf(),
                    source,
                })
        })
        .collect::<Result<_, _>>()?;
    paths.retain(|path| {
        path.is_file()
            && matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yaml" | "yml")
            )
    });
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| CreaturesLoadError::UnnamedFile {
                    path: path.to_path_buf(),
                })?
                .to_string();
            let contents =
                fs::read_to_string(&path).map_err(|source| CreaturesLoadError::ReadError {
                    path: path.to_path_buf(),
                    source,
                })?;
            let raw: RawCreature = serde_yaml::from_str(&contents).map_err(|source| {
                CreaturesLoadError::ParseError {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            Ok((id, Arc::new(raw.into_kind())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CONFIG;
    use crate::entities::agent::Agent;
    use crate::entities::position::Position;
    use crate::persistence::items::ITEM_CONFIGS;

    #[test]
    fn every_shipped_creature_walks_at_its_tibia_speed() {
        let creatures = load_creatures(&CONFIG.creatures_dir_path).unwrap();

        for (name, expected_ms) in [("Demon", 500), ("Dragon", 700), ("Elf", 650)] {
            let kind = creatures
                .values()
                .find(|kind| kind.name == name)
                .unwrap_or_else(|| panic!("{name} is not among the shipped creatures"));
            let agent = Agent::from_creature_kind(kind.clone(), Position::new(1028, 128, 7));

            assert_eq!(
                agent.calculate_walk_ticks(150, false) * 50,
                expected_ms,
                "{name} walks a normal tile in {expected_ms}ms in the reference"
            );
        }
    }

    #[test]
    fn every_shipped_loot_and_corpse_id_resolves_to_an_item() {
        let creatures = load_creatures(&CONFIG.creatures_dir_path).unwrap();
        assert!(
            !creatures.is_empty(),
            "loaded no creatures at all; the rest of this test would pass vacuously"
        );

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

    /// A creature's kind id is its file name, so a rename or a typo severs every spawn
    /// point naming it. The spawner logs and skips, which is quiet enough to ship.
    #[test]
    fn every_shipped_spawn_names_a_creature_that_exists() {
        let creatures = load_creatures(&CONFIG.creatures_dir_path).unwrap();
        let spawns = crate::persistence::spawns::load_spawns(&CONFIG.spawns_file_path).unwrap();

        let unknown: Vec<&str> = spawns
            .iter()
            .map(|spawn| spawn.kind.as_str())
            .filter(|kind| !creatures.contains_key(*kind))
            .collect();

        assert!(
            unknown.is_empty(),
            "spawns.yaml names creatures with no file in {}: {unknown:?}",
            CONFIG.creatures_dir_path
        );
    }
}
