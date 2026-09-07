use std::sync::Arc;

use crate::{
    entities::{
        agent::{AgentId, AgentKey},
        combat::CombatElement,
        effects::{AreaShape, EffectId, MissileId},
        position::Position,
        vocation::Vocation,
    },
    game::TickDelta,
};

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct SpellId(pub u16);

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
pub struct SpellAttack {
    pub target: SpellTargetMode,
    pub element: CombatElement,
    pub base_power: u16,
    pub level_factor: u16,
    pub magic_factor: u16,
    pub effect_id: EffectId,
    pub missile_id: Option<MissileId>,
}

#[derive(Debug)]
pub enum SpellEffect {
    Attack(SpellAttack),
}

#[derive(Debug)]
pub enum SpellTargetMode {
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

/// Client target enum
#[derive(Debug, Clone)]
pub enum SpellTarget {
    None,
    Agent(AgentId),
    Position(Position),
}

/// World target enum
#[derive(Debug, Clone)]
pub enum CastTarget {
    None,
    Agent(AgentKey),
    Position(Position),
}
