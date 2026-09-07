use crate::entities::position::Position;

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
    pub delta: Vec<(i8, i8)>,
}
