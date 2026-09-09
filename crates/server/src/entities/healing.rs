use smallvec::SmallVec;

use crate::entities::{agent::AgentKey, effects::AreaEffect};

pub struct HealPlan {
    pub caster: AgentKey,
    pub restores: SmallVec<[(AgentKey, Restore); 1]>,
    pub area_effect: Option<AreaEffect>,
}

pub struct Restore {
    pub life: Option<u32>,
    pub mana: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum RestoreType {
    Life,
    Mana,
}
