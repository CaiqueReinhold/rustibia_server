use std::collections::HashMap;

use thiserror::Error;

use crate::entities::{
    player::{Player, PlayerId, Pool, Skill},
    position::Position,
};

#[derive(Error, Debug)]
pub enum PlayerRepositoryError {
    #[error("Player not found")]
    NotFound,
}

pub struct PlayerRepository {}

impl PlayerRepository {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn get_by_id(&self, id: PlayerId) -> Result<Player, PlayerRepositoryError> {
        if id == 1 {
            Ok(Player {
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
                experience: Pool {
                    current: 0,
                    maximum: 100,
                },
                inventory: HashMap::new(),
                level: 1,
                magic: Skill {
                    value: 1,
                    current_ticks: 0,
                    max_ticks: 100,
                },
                meele: Skill {
                    value: 1,
                    current_ticks: 0,
                    max_ticks: 100,
                },
                life: Pool {
                    current: 100,
                    maximum: 100,
                },
                mana: Pool {
                    current: 100,
                    maximum: 100,
                },
                base_speed: 120,
                outfit: 133,
            })
        } else {
            Err(PlayerRepositoryError::NotFound)
        }
    }

    pub async fn _save(&self, _player: &Player) -> Result<(), PlayerRepositoryError> {
        todo!()
    }
}
