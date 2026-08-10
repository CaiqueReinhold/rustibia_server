use std::collections::HashMap;

use crate::entities::agent::{OutfitColors, OutfitId, Pool};
use crate::entities::skills::{SkillType, SkillValue};

pub type CreatureKindId = String;

#[derive(Clone, Debug)]
pub struct CreatureKind {
    pub name: String,
    pub life: Pool,
    pub outfit: (OutfitId, OutfitColors),
    pub speed: u16,
    pub skills: HashMap<SkillType, SkillValue>,
}
