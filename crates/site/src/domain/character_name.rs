use crate::error::AppError;

pub const MIN_LENGTH: usize = 2;
pub const MAX_LENGTH: usize = 29;

/// Validates a character name and returns it trimmed.
///
/// Rules: 2–29 characters, letters and single interior spaces only, no leading or
/// trailing whitespace, no doubled spaces. Uniqueness is NOT checked here — that is
/// enforced by `players_name_lower_idx` and pre-checked in `db::characters` so the
/// user gets a readable message instead of a 500.
pub fn validate(raw: &str) -> Result<String, AppError> {
    let invalid = |message: &str| Err(AppError::Validation(message.to_string()));

    if raw != raw.trim() {
        return invalid("Character names cannot start or end with a space.");
    }
    let name = raw;

    if name.chars().count() < MIN_LENGTH {
        return invalid("Character names must be at least 2 characters long.");
    }
    if name.chars().count() > MAX_LENGTH {
        return invalid("Character names may be at most 29 characters long.");
    }
    if name.contains("  ") {
        return invalid("Character names cannot contain two spaces in a row.");
    }
    if !name.chars().all(|c| c.is_alphabetic() || c == ' ') {
        return invalid("Character names may only contain letters and spaces.");
    }

    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_message(raw: &str) -> String {
        match validate(raw) {
            Err(AppError::Validation(m)) => m,
            other => panic!("expected a validation error for {raw:?}, got {other:?}"),
        }
    }

    #[test]
    fn accepts_a_simple_name() {
        assert_eq!(validate("Rizael").unwrap(), "Rizael");
    }

    #[test]
    fn accepts_a_name_with_a_single_interior_space() {
        assert_eq!(validate("Sir Rizael").unwrap(), "Sir Rizael");
    }

    #[test]
    fn rejects_leading_or_trailing_whitespace() {
        assert!(!err_message(" Rizael").is_empty());
        assert!(!err_message("Rizael ").is_empty());
    }

    #[test]
    fn rejects_doubled_spaces() {
        assert!(err_message("Sir  Rizael").contains("two spaces"));
    }

    #[test]
    fn rejects_digits_and_punctuation() {
        assert!(err_message("Riz4el").contains("letters"));
        assert!(err_message("Rizael!").contains("letters"));
        assert!(err_message("Riz_ael").contains("letters"));
    }

    #[test]
    fn rejects_names_that_are_too_short_or_too_long() {
        assert!(err_message("R").contains("at least"));
        assert!(err_message(&"R".repeat(30)).contains("at most"));
    }

    #[test]
    fn accepts_the_exact_length_boundaries() {
        assert!(validate("Ri").is_ok());
        assert!(validate(&"R".repeat(29)).is_ok());
    }

    #[test]
    fn counts_characters_not_bytes() {
        // "Ää" is 2 characters but 4 bytes. A byte-length check would wrongly accept
        // it as long enough while rejecting a 15-character accented name as too long.
        assert!(validate("Ää").is_ok());
        assert!(validate(&"ä".repeat(29)).is_ok());
        assert!(validate(&"ä".repeat(30)).is_err());
    }
}
