/// A visual effect in the client's appearance data — a splash, a puff, a sparkle.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct EffectId(pub u16);

/// A projectile in the client's appearance data — an arrow, a bolt, a spell missile.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MissileId(pub u16);
