use std::collections::HashMap;

use thiserror::Error;

use crate::entities::{
    agent::{Facing, OutfitColors, OutfitId, Pool},
    items::Item,
    player::{InventorySlot, Player, PlayerId},
    position::Position,
    skills::{SkillType, SkillValue},
};

#[derive(Error, Debug)]
pub enum PlayerRepositoryError {
    #[error("Player not found")]
    NotFound,
}

pub struct PlayerSnapshot {
    pub id: PlayerId,
    pub position: Position,
    pub origin: Position,
    pub facing: Facing,
    pub name: String,
    pub life: Pool,
    pub mana: Pool,
    pub capacity: Pool,
    pub outfit: (OutfitId, OutfitColors),
    pub skills: HashMap<SkillType, SkillValue>,
    pub inventory: HashMap<InventorySlot, Item>,
}

pub struct PlayerRepository {}

impl PlayerRepository {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn get_by_id(&self, id: PlayerId) -> Result<PlayerSnapshot, PlayerRepositoryError> {
        if id == 1 {
            let mut player = PlayerSnapshot {
                id: 1,
                name: "Rizael".to_string(),
                position: Position {
                    x: 1028,
                    y: 1028,
                    z: 7,
                },
                origin: Position {
                    x: 1028,
                    y: 1028,
                    z: 7,
                },
                facing: Facing::South,
                life: Pool {
                    current: 100,
                    maximum: 100,
                },
                mana: Pool {
                    current: 100,
                    maximum: 100,
                },
                capacity: Pool {
                    current: 0,
                    maximum: 40000,
                },
                outfit: (133, (1, 2, 3, 4)),
                inventory: HashMap::new(),
                skills: HashMap::new(),
            };
            player.skills.insert(
                SkillType::Level,
                SkillValue {
                    value: 1,
                    current_ticks: 0,
                    max_ticks: 100,
                },
            );
            player.skills.insert(
                SkillType::Speed,
                SkillValue {
                    value: 120,
                    current_ticks: 0,
                    max_ticks: 0,
                },
            );
            Ok(player)
        } else {
            Err(PlayerRepositoryError::NotFound)
        }
    }

    pub async fn _save(&self, _player: &Player) -> Result<(), PlayerRepositoryError> {
        todo!()
    }
}
