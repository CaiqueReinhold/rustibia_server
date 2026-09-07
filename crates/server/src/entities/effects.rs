use std::sync::Arc;

use crate::entities::{agent::Facing, position::Position};

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct EffectId(pub u16);

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MissileId(pub u16);

#[derive(Debug, Clone)]
pub struct Missile {
    pub missile_id: MissileId,
    pub from: Position,
    pub to: Position,
}

#[derive(Debug)]
pub struct AreaEffect {
    pub effect_id: EffectId,
    pub origin: Position,
    pub shape: Arc<AreaShape>,
}

pub type AreaShapeId = String;

#[derive(Debug)]
pub struct AreaShape {
    delta: [Box<[(i8, i8)]>; 4],
}

impl AreaShape {
    fn rotate_cw(tiles: &[(i8, i8)]) -> Box<[(i8, i8)]> {
        tiles.iter().map(|&(dx, dy)| (-dy, dx)).collect()
    }

    pub fn new(north_facing: Box<[(i8, i8)]>) -> Self {
        let east_facing = Self::rotate_cw(&north_facing);
        let south_facing = Self::rotate_cw(&east_facing);
        let west_facing = Self::rotate_cw(&south_facing);
        AreaShape {
            delta: [north_facing, east_facing, south_facing, west_facing],
        }
    }

    pub fn get_delta(&self, facing: Facing) -> &[(i8, i8)] {
        &self.delta[match facing {
            Facing::North => 0,
            Facing::East => 1,
            Facing::South => 2,
            Facing::West => 3,
        }]
    }
}
