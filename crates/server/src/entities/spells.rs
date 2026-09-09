use std::sync::Arc;

use strum::EnumCount;

use crate::{
    entities::{
        agent::{AgentId, AgentKey},
        combat::CombatElement,
        effects::{AreaShape, EffectId, MissileId},
        position::Position,
        vocation::Vocation,
    },
    game::{TickDelta, config::GAME_CONFIG},
};

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct SpellId(pub u16);

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize, EnumCount)]
#[serde(rename_all = "snake_case")]
pub enum SpellGroup {
    Attack,
    Healing,
    Support,
}

impl SpellGroup {
    pub fn index(&self) -> usize {
        *self as usize
    }

    pub fn cooldown(&self) -> TickDelta {
        match self {
            SpellGroup::Attack => GAME_CONFIG.combat.attack_group_cooldown,
            SpellGroup::Healing => GAME_CONFIG.combat.healing_group_cooldown,
            SpellGroup::Support => GAME_CONFIG.combat.support_group_cooldown,
        }
    }
}

#[derive(Debug)]
pub struct Spell {
    pub id: SpellId,
    pub name: String,
    pub words: String,
    pub group: SpellGroup,
    pub group_cooldown: Option<TickDelta>,
    pub cooldown: TickDelta,
    pub mana: u32,
    pub level: u16,
    pub vocations: Vec<Vocation>,
    pub effects: Vec<SpellEffect>,
}

/// `level_factor` and `magic_factor` are percentages of `base_power`; `spread` is the
/// fraction either side of the centre the roll spans.
#[derive(Debug)]
pub struct PowerCurve {
    pub base_power: f32,
    pub level_factor: f32,
    pub magic_factor: f32,
    pub spread: f32,
}

#[derive(Debug)]
pub struct SpellAttack {
    pub target: SpellTargetMode,
    pub element: CombatElement,
    pub power: PowerCurve,
    pub effect_id: EffectId,
    pub missile_id: Option<MissileId>,
}

#[derive(Debug)]
pub struct SpellHealing {
    pub target: SpellTargetMode,
    pub power: PowerCurve,
}

#[derive(Debug)]
pub enum SpellEffect {
    Attack(SpellAttack),
    Healing(SpellHealing),
}

#[derive(Debug)]
pub enum SpellTargetMode {
    /// Accepts only CastTarget::None
    Caster,
    /// Accepts only CastTarget::None, the target is always
    /// agent.target()
    Target { range: u16 },
    /// if origin is caster accepts only CastTarget::None
    /// otherwise accepts only CastTarget::Agent or CastTarget::Position
    Area {
        origin: AreaOrigin,
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
