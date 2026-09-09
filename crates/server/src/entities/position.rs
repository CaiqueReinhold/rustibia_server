use std::{
    fmt::Display,
    ops::{Add, Sub},
};

use crate::{
    constants::{
        items::{CONTAINER_COORD_FLAG, INVENTORY_COORD_FLAG},
        view::{PLAYER_VIEWPORT_HEIGHT, PLAYER_VIEWPORT_WIDTH},
    },
    entities::{agent::AgentKey, inventory::InventorySlot},
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

    pub fn checked_offset(&self, dx: i32, dy: i32) -> Option<Position> {
        let x = u16::try_from(self.x as i32 + dx).ok()?;
        let y = u16::try_from(self.y as i32 + dy).ok()?;
        Some(Position::new(x, y, self.z))
    }
}

impl Add<Direction> for Position {
    type Output = Position;

    fn add(self, rhs: Direction) -> Self::Output {
        let (dx, dy) = rhs.delta();
        Self {
            x: self.x.saturating_add_signed(dx as i16),
            y: self.y.saturating_add_signed(dy as i16),
            z: self.z,
        }
    }
}

impl Sub<Direction> for Position {
    type Output = Position;

    fn sub(self, rhs: Direction) -> Self::Output {
        let (dx, dy) = rhs.delta();
        Self {
            x: self.x.saturating_sub_signed(dx as i16),
            y: self.y.saturating_sub_signed(dy as i16),
            z: self.z,
        }
    }
}

impl Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
    }
}

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
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

    pub fn delta(&self) -> (i32, i32) {
        match self {
            Direction::North => (0, -1),
            Direction::NorthEast => (1, -1),
            Direction::East => (1, 0),
            Direction::SouthEast => (1, 1),
            Direction::South => (0, 1),
            Direction::SouthWest => (-1, 1),
            Direction::West => (-1, 0),
            Direction::NorthWest => (-1, -1),
        }
    }

    pub fn from_step(dx: i32, dy: i32) -> Option<Direction> {
        ALL_DIRECTIONS
            .into_iter()
            .find(|direction| direction.delta() == (dx, dy))
    }
}

pub const ALL_DIRECTIONS: [Direction; 8] = [
    Direction::North,
    Direction::NorthEast,
    Direction::East,
    Direction::SouthEast,
    Direction::South,
    Direction::SouthWest,
    Direction::West,
    Direction::NorthWest,
];

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
        Self::radius(pos, (half_w, half_h))
    }

    pub fn radius(pos: &Position, radius: (u16, u16)) -> Self {
        Rect {
            min: Point {
                x: pos.x.saturating_sub(radius.0),
                y: pos.y.saturating_sub(radius.1),
            },
            max: Point {
                x: pos.x.saturating_add(radius.0),
                y: pos.y.saturating_add(radius.1),
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
    fn a_player_viewport_at_the_map_edge_clamps_instead_of_overflowing() {
        let rect = Rect::player_viewport(&Position::new(u16::MAX, u16::MAX, 7));

        assert_eq!((rect.max_x(), rect.max_y()), (u16::MAX, u16::MAX));
        assert!(rect.contains(&Position::new(u16::MAX, u16::MAX, 7)));

        let origin = Rect::player_viewport(&Position::new(0, 0, 7));

        assert_eq!((origin.min_x(), origin.min_y()), (0, 0));
        assert!(origin.contains(&Position::new(0, 0, 7)));
    }

    #[test]
    fn rect_contains_ignores_the_floor() {
        let rect = Rect::new(10, 20, 14, 24);

        assert!(rect.contains(&Position::new(12, 22, 0)));
        assert!(rect.contains(&Position::new(12, 22, 15)));
    }
}
