use std::ops::Add;

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

#[derive(Clone, Debug)]
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
