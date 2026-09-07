use std::sync::Arc;

use crate::{
    entities::{
        combat::CombatElement,
        effects::{AreaShape, EffectId, MissileId},
        vocation::Vocation,
    },
    game::TickDelta,
};

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct SpellId(u16);

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpellGroup {
    Attack,
    Healing,
    Support,
}

#[derive(Debug)]
pub struct Spell {
    pub id: SpellId,
    pub name: String,
    pub group: SpellGroup,
    pub group_cooldown: Option<TickDelta>,
    pub cooldown: TickDelta,
    pub mana: u32,
    pub level: u32,
    pub vocations: Vec<Vocation>,
    pub effects: Vec<SpellEffect>,
}

#[derive(Debug)]
pub enum SpellEffect {
    Attack {
        target: SpellTarget,
        element: CombatElement,
        base_power: u16,
        level_factor: u16,
        magic_factor: u16,
        effect_id: EffectId,
        missile_id: Option<MissileId>,
    },
}

#[derive(Debug)]
pub enum SpellTarget {
    Caster,
    Target,
    Area {
        origin: AreaOrigin,
        rotate: bool,
        shape: Arc<AreaShape>,
    },
}

#[derive(Debug)]
pub enum AreaOrigin {
    Caster,
    Target,
}
