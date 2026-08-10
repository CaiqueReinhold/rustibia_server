use std::fs;
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

use crate::entities::creature::CreatureKindId;
use crate::entities::position::Position;
use crate::game::Tick;

#[derive(Error, Debug)]
pub enum SpawnsLoadError {
    #[error("I/O error: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    ParseError(#[from] serde_yaml::Error),
}

#[derive(Clone, Debug, Deserialize)]
pub struct SpawnPoint {
    pub position: Position,
    pub kind: CreatureKindId,
    pub respawn_ticks: Tick,
}

#[derive(Deserialize)]
struct SpawnsFile {
    spawns: Vec<SpawnPoint>,
}

pub fn load_spawns(path: impl AsRef<Path>) -> Result<Vec<SpawnPoint>, SpawnsLoadError> {
    let contents = fs::read_to_string(path)?;
    let file: SpawnsFile = serde_yaml::from_str(&contents)?;
    Ok(file.spawns)
}
