use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use thiserror::Error;

use crate::entities::agent::{OutfitColors, OutfitId, Pool};
use crate::entities::creature::{CreatureKind, CreatureKindId};

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
struct RawCreature {
    name: String,
    life: u32,
    speed: u16,
    outfit: RawOutfit,
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
                outfit: (raw.outfit.id, raw.outfit.colors),
                speed: raw.speed,
                skills: HashMap::new(),
            };
            (id, Arc::new(kind))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_demon_from_assets() {
        let creatures = load_creatures("assets/creatures.yaml").unwrap();
        let demon = creatures.get("demon").expect("demon kind missing");
        assert_eq!(demon.name, "Demon");
        assert_eq!(demon.life.maximum, 8200);
        assert_eq!(demon.speed, 230);
        assert_eq!(demon.outfit.0, 35);
    }
}
