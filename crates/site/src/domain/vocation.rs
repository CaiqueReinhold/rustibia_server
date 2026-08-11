use crate::error::AppError;

/// Stored in `players.vocation` as a SMALLINT. The four classic Tibia vocations.
///
/// All four start with identical stats today — vocation is a stored label, and the
/// balance work happens game-side later. The discriminants are part of the database
/// contract: the game server will read these same numbers once it grows vocation
/// support, so they must never be reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vocation {
    Knight = 0,
    Paladin = 1,
    Sorcerer = 2,
    Druid = 3,
}

impl Vocation {
    pub const ALL: [Vocation; 4] = [
        Vocation::Knight,
        Vocation::Paladin,
        Vocation::Sorcerer,
        Vocation::Druid,
    ];

    pub fn as_i16(self) -> i16 {
        self as i16
    }

    pub fn from_i16(value: i16) -> Result<Self, AppError> {
        match value {
            0 => Ok(Vocation::Knight),
            1 => Ok(Vocation::Paladin),
            2 => Ok(Vocation::Sorcerer),
            3 => Ok(Vocation::Druid),
            other => Err(AppError::Validation(format!("Unknown vocation: {other}."))),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Vocation::Knight => "Knight",
            Vocation::Paladin => "Paladin",
            Vocation::Sorcerer => "Sorcerer",
            Vocation::Druid => "Druid",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_i16() {
        for vocation in Vocation::ALL {
            assert_eq!(Vocation::from_i16(vocation.as_i16()).unwrap(), vocation);
        }
    }

    #[test]
    fn discriminants_are_pinned_to_the_database_contract() {
        assert_eq!(Vocation::Knight.as_i16(), 0);
        assert_eq!(Vocation::Paladin.as_i16(), 1);
        assert_eq!(Vocation::Sorcerer.as_i16(), 2);
        assert_eq!(Vocation::Druid.as_i16(), 3);
    }

    #[test]
    fn rejects_an_out_of_range_value() {
        assert!(matches!(
            Vocation::from_i16(9),
            Err(AppError::Validation(_))
        ));
        assert!(matches!(
            Vocation::from_i16(-1),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn every_vocation_has_a_display_name() {
        for vocation in Vocation::ALL {
            assert!(!vocation.name().is_empty());
        }
    }
}
