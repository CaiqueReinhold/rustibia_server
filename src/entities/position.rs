use std::ops::{Add, Sub};

use crate::{
    constants::{
        CONTAINER_COORD_FLAG, INVENTORY_COORD_FLAG, PLAYER_VIEWPORT_HEIGHT, PLAYER_VIEWPORT_WIDTH,
    },
    entities::{agent::AgentKey, player::InventorySlot},
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
pub struct Position {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl Position {
    pub fn is_container_coord(&self) -> bool {
        self.x == CONTAINER_COORD_FLAG
    }

    pub fn is_inventory_coord(&self) -> bool {
        self.x == INVENTORY_COORD_FLAG
    }

    pub fn in_viewport(&self, other: &Position) -> bool {
        let half_x = (PLAYER_VIEWPORT_WIDTH / 2) as u32;
        let half_y = (PLAYER_VIEWPORT_HEIGHT / 2) as u32;
        let start_x = self.x.saturating_sub(half_x);
        let end_x = self.x + half_x;
        let start_y = self.y.saturating_sub(half_y);
        let end_y = self.y + half_y;
        other.x >= start_x && other.x <= end_x && other.y >= start_y && other.y <= end_y
    }

    pub fn is_adjacent(&self, other: &Position) -> bool {
        if self.z != other.z {
            return false;
        }
        let dx = (self.x as i64 - other.x as i64).abs();
        let dy = (self.y as i64 - other.y as i64).abs();
        dx <= 1 && dy <= 1
    }
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

impl Sub<Direction> for Position {
    type Output = Position;

    fn sub(self, rhs: Direction) -> Self::Output {
        match rhs {
            Direction::North => Self {
                x: self.x,
                y: self.y + 1,
                z: self.z,
            },
            Direction::South => Self {
                x: self.x,
                y: self.y - 1,
                z: self.z,
            },
            Direction::East => Self {
                x: self.x - 1,
                y: self.y,
                z: self.z,
            },
            Direction::West => Self {
                x: self.x + 1,
                y: self.y,
                z: self.z,
            },
            Direction::NorthEast => Self {
                x: self.x - 1,
                y: self.y + 1,
                z: self.z,
            },
            Direction::NorthWest => Self {
                x: self.x + 1,
                y: self.y + 1,
                z: self.z,
            },
            Direction::SouthEast => Self {
                x: self.x - 1,
                y: self.y - 1,
                z: self.z,
            },
            Direction::SouthWest => Self {
                x: self.x + 1,
                y: self.y - 1,
                z: self.z,
            },
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub enum Direction {
    North,
    East,
    West,
    South,
    NorthEast,
    SouthEast,
    NorthWest,
    SouthWest,
}

impl Direction {
    pub fn is_diagonal(&self) -> bool {
        matches!(
            self,
            Direction::NorthEast
                | Direction::NorthWest
                | Direction::SouthEast
                | Direction::SouthWest
        )
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ItemPlacement {
    Map(Position),
    Inventory(InventorySlot, AgentKey),
}
