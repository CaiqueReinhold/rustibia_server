#[derive(Clone, Debug)]
pub struct SkillValue {
    pub value: u16,
    pub current_ticks: u64,
    pub max_ticks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SkillType {
    Level,
    Speed,
}
