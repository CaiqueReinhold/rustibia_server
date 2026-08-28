#[derive(Clone, Debug)]
pub struct SkillValue {
    pub value: u16,
    pub current_ticks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SkillType {
    Level,
    Sword,
    Club,
    Axe,
    Distance,
    Magic,
    Shielding,
}

impl SkillType {
    pub fn as_id(&self) -> u8 {
        match self {
            SkillType::Level => 0,
            SkillType::Axe => 1,
            SkillType::Club => 2,
            SkillType::Sword => 3,
            SkillType::Distance => 4,
            SkillType::Magic => 5,
            SkillType::Shielding => 6,
        }
    }

    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(SkillType::Level),
            1 => Some(SkillType::Axe),
            2 => Some(SkillType::Club),
            3 => Some(SkillType::Sword),
            4 => Some(SkillType::Distance),
            5 => Some(SkillType::Magic),
            6 => Some(SkillType::Shielding),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client repeats these ids and nothing links the two — separate
    /// repositories, no shared crate. The matching assertion lives in the
    /// client's `game_ui/skills/mod.rs`. A divergence is silent: every id still
    /// decodes to some skill, so the bars are simply labelled wrong.
    #[test]
    fn the_wire_ids_are_the_stored_ids() {
        assert_eq!(SkillType::Level.as_id(), 0);
        assert_eq!(SkillType::Axe.as_id(), 1);
        assert_eq!(SkillType::Club.as_id(), 2);
        assert_eq!(SkillType::Sword.as_id(), 3);
        assert_eq!(SkillType::Distance.as_id(), 4);
        assert_eq!(SkillType::Magic.as_id(), 5);
        assert_eq!(SkillType::Shielding.as_id(), 6);
    }

    #[test]
    fn every_id_round_trips_and_unknown_ids_are_rejected() {
        for skill in [
            SkillType::Level,
            SkillType::Axe,
            SkillType::Club,
            SkillType::Sword,
            SkillType::Distance,
            SkillType::Magic,
            SkillType::Shielding,
        ] {
            assert_eq!(SkillType::from_id(skill.as_id()), Some(skill.clone()));
        }
        assert_eq!(SkillType::from_id(7), None);
        assert_eq!(SkillType::from_id(255), None);
    }
}
