use crate::error::AppError;

/// Stored in `players.sex` as a SMALLINT. Selects the starting outfit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sex {
    Female = 0,
    Male = 1,
}

impl Sex {
    pub const ALL: [Sex; 2] = [Sex::Female, Sex::Male];

    pub fn as_i16(self) -> i16 {
        self as i16
    }

    pub fn from_i16(value: i16) -> Result<Self, AppError> {
        match value {
            0 => Ok(Sex::Female),
            1 => Ok(Sex::Male),
            other => Err(AppError::Validation(format!("Unknown sex: {other}."))),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sex::Female => "Female",
            Sex::Male => "Male",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_i16() {
        for sex in Sex::ALL {
            assert_eq!(Sex::from_i16(sex.as_i16()).unwrap(), sex);
        }
    }

    #[test]
    fn discriminants_are_pinned_to_the_database_contract() {
        assert_eq!(Sex::Female.as_i16(), 0);
        assert_eq!(Sex::Male.as_i16(), 1);
    }

    #[test]
    fn rejects_an_out_of_range_value() {
        assert!(matches!(Sex::from_i16(2), Err(AppError::Validation(_))));
    }
}
