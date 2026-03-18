use smallvec::SmallVec;
use std::ops::Add;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;

use crate::constants::MAX_VISIBLE_ITEMS;
use crate::entities::agent::{Agent, AgentKey};
use crate::entities::items::Item;
use crate::messages::Direction;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
pub struct Position {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl Add<Direction> for Position {
    type Output = Position;

    fn add(self, rhs: Direction) -> Self::Output {
        match rhs {
            Direction::North => Self {
                x: self.x,
                y: self.y - 1,
                z: self.z,
            },
            Direction::South => Self {
                x: self.x,
                y: self.y + 1,
                z: self.z,
            },
            Direction::East => Self {
                x: self.x + 1,
                y: self.y,
                z: self.z,
            },
            Direction::West => Self {
                x: self.x - 1,
                y: self.y,
                z: self.z,
            },
            Direction::NorthEast => Self {
                x: self.x + 1,
                y: self.y - 1,
                z: self.z,
            },
            Direction::NorthWest => Self {
                x: self.x - 1,
                y: self.y - 1,
                z: self.z,
            },
            Direction::SouthEast => Self {
                x: self.x + 1,
                y: self.y + 1,
                z: self.z,
            },
            Direction::SouthWest => Self {
                x: self.x - 1,
                y: self.y + 1,
                z: self.z,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct MapTile {
    items: SmallVec<[Item; MAX_VISIBLE_ITEMS]>,
    agents: SmallVec<[AgentKey; 1]>,
}

#[derive(Error, Debug)]
pub enum MapError {
    #[error("Tile position does not exist")]
    TileDoesNotExist,
    #[error("Entity does not exist at this position")]
    EntityNotInPosition,
}

#[derive(Debug, Clone)]
pub struct GameMap {
    tiles: HashMap<Position, MapTile>,
}

impl MapTile {
    pub fn new() -> Self {
        MapTile {
            items: SmallVec::new(),
            agents: SmallVec::new(),
        }
    }

    pub fn push_item(&mut self, item: Item) {
        self.items.push(item);
    }
}

impl GameMap {
    pub fn new() -> Self {
        GameMap {
            tiles: HashMap::new(),
        }
    }

    pub fn insert_tile(&mut self, pos: Position, tile: MapTile) {
        self.tiles.insert(pos, tile);
    }

    fn get_tile_mut(&mut self, pos: &Position) -> Result<&mut MapTile, MapError> {
        if let Some(tile) = self.tiles.get_mut(pos) {
            return Ok(tile);
        }
        Err(MapError::TileDoesNotExist)
    }

    pub fn add_actor(&mut self, pos: &Position, handle: AgentKey) -> Result<(), MapError> {
        let tile = self.get_tile_mut(pos)?;
        tile.agents.push(handle);
        Ok(())
    }

    pub fn remove_actor(&mut self, pos: &Position, handle: AgentKey) -> Result<(), MapError> {
        let tile = self.get_tile_mut(pos)?;
        let index = tile
            .agents
            .iter()
            .enumerate()
            .find(|(_, act)| **act == handle)
            .map(|(i, _)| i);
        if let Some(index) = index {
            tile.agents.remove(index);
            return Ok(());
        }
        Err(MapError::EntityNotInPosition)
    }

    pub fn can_move(&self, _pos: &Position, _actor: &Agent) -> bool {
        true
    }

    pub fn tile_speed(&self, _pos: &Position) -> u8 {
        0
    }
}

pub struct SharedGameMap {
    inner: Arc<RwLock<GameMap>>,
}

pub struct DoubleBufferedMap {
    maps: [Arc<RwLock<GameMap>>; 2],
    index: usize,
}

impl DoubleBufferedMap {
    pub fn new(map: GameMap) -> Self {
        Self {
            maps: [
                Arc::new(RwLock::new(map.clone())),
                Arc::new(RwLock::new(map)),
            ],
            index: 0,
        }
    }

    pub fn write(&mut self) -> RwLockWriteGuard<'_, GameMap> {
        self.maps[self.index].write().unwrap()
    }

    pub fn read(&self) -> RwLockReadGuard<'_, GameMap> {
        self.maps[self.index].read().unwrap()
    }

    pub fn as_shared(&self) -> SharedGameMap {
        let inner = self.maps[self.index - 1].clone();
        SharedGameMap { inner }
    }

    pub fn swap(&mut self) {
        let _unused = self.maps[1 - self.index].write().unwrap();
        self.index = 1 - self.index;
    }
}
