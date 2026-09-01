use std::{
    fmt::Display,
    ops::{Add, Sub},
};

use crate::{
    constants::{
        CONTAINER_COORD_FLAG, INVENTORY_COORD_FLAG, PLAYER_VIEWPORT_HEIGHT, PLAYER_VIEWPORT_WIDTH,
    },
    entities::{agent::AgentKey, player::InventorySlot},
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Default, serde::Deserialize)]
pub struct Position {
    pub x: u16,
    pub y: u16,
    pub z: u8,
}

impl Position {
    pub fn new(x: u16, y: u16, z: u8) -> Self {
        Self { x, y, z }
    }

    pub fn is_container_coord(&self) -> bool {
        self.x == CONTAINER_COORD_FLAG
    }

    pub fn is_inventory_coord(&self) -> bool {
        self.x == INVENTORY_COORD_FLAG
    }

    pub fn in_viewport(&self, other: &Position) -> bool {
        let half_x = (PLAYER_VIEWPORT_WIDTH / 2) as u16;
        let half_y = (PLAYER_VIEWPORT_HEIGHT / 2) as u16;
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

    pub fn placement_is_adjacent(&self, placement: &ItemPlacement) -> bool {
        match placement {
            ItemPlacement::Map(pos) => self.is_adjacent(pos),
            ItemPlacement::Inventory(..) => true,
        }
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

impl Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
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

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Point {
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Rect {
    min: Point,
    max: Point,
}

impl Rect {
    pub fn new(min_x: u16, min_y: u16, max_x: u16, max_y: u16) -> Self {
        Rect {
            min: Point { x: min_x, y: min_y },
            max: Point { x: max_x, y: max_y },
        }
    }

    pub fn player_viewport(pos: &Position) -> Self {
        let half_w = (PLAYER_VIEWPORT_WIDTH / 2) as u16;
        let half_h = (PLAYER_VIEWPORT_HEIGHT / 2) as u16;
        let x = pos.x;
        let y = pos.y;

        Rect {
            min: Point {
                x: x.saturating_sub(half_w),
                y: y.saturating_sub(half_h),
            },
            max: Point {
                x: x + half_w,
                y: y + half_h,
            },
        }
    }

    /// x/y only — `Rect` has no floor, so `z` is the caller's clause.
    pub fn contains(&self, pos: &Position) -> bool {
        pos.x >= self.min.x && pos.x <= self.max.x && pos.y >= self.min.y && pos.y <= self.max.y
    }

    pub fn min_x(&self) -> u16 {
        self.min.x
    }

    pub fn min_y(&self) -> u16 {
        self.min.y
    }

    pub fn max_x(&self) -> u16 {
        self.max.x
    }

    pub fn max_y(&self) -> u16 {
        self.max.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_its_edges_but_not_beyond() {
        let rect = Rect::new(10, 20, 14, 24);

        assert!(rect.contains(&Position::new(12, 22, 7)));
        assert!(rect.contains(&Position::new(10, 20, 7)));
        assert!(rect.contains(&Position::new(14, 24, 7)));
        assert!(!rect.contains(&Position::new(9, 22, 7)));
        assert!(!rect.contains(&Position::new(15, 22, 7)));
        assert!(!rect.contains(&Position::new(12, 25, 7)));
    }

    #[test]
    fn rect_contains_ignores_the_floor() {
        let rect = Rect::new(10, 20, 14, 24);

        assert!(rect.contains(&Position::new(12, 22, 0)));
        assert!(rect.contains(&Position::new(12, 22, 15)));
    }
}
