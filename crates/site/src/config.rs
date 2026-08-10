use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct StartingSkill {
    pub skill_type: i16,
    pub value: i16,
    pub current_ticks: i64,
    pub max_ticks: i64,
}

/// Every new character starts identical regardless of vocation, per the design.
#[derive(Debug, Deserialize, Clone)]
pub struct NewCharacterConfig {
    pub pos_x: i32,
    pub pos_y: i32,
    pub pos_z: i16,
    pub facing: i16,
    pub life: i32,
    pub mana: i32,
    pub capacity: i32,
    pub outfit_id_female: i16,
    pub outfit_id_male: i16,
    pub outfit_head: i16,
    pub outfit_body: i16,
    pub outfit_legs: i16,
    pub outfit_feet: i16,
    pub starting_skills: Vec<StartingSkill>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SiteConfig {
    pub session_ttl_days: i64,
    pub auth_token_ttl_seconds: i64,
    pub min_password_length: usize,
    pub new_character: NewCharacterConfig,
}

impl SiteConfig {
    pub fn from_yaml(contents: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(contents)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path.as_ref())
            .map_err(|e| ConfigError::Read(path.as_ref().display().to_string(), e))?;
        Ok(Self::from_yaml(&contents)?)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config at {0}: {1}")]
    Read(String, std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] serde_yaml::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_config() {
        let cfg = SiteConfig::from_yaml(
            "session_ttl_days: 7\n\
             auth_token_ttl_seconds: 60\n\
             min_password_length: 8\n\
             new_character:\n\
             \x20\x20pos_x: 1028\n\
             \x20\x20pos_y: 1028\n\
             \x20\x20pos_z: 7\n\
             \x20\x20facing: 2\n\
             \x20\x20life: 150\n\
             \x20\x20mana: 0\n\
             \x20\x20capacity: 400\n\
             \x20\x20outfit_id_female: 136\n\
             \x20\x20outfit_id_male: 128\n\
             \x20\x20outfit_head: 78\n\
             \x20\x20outfit_body: 69\n\
             \x20\x20outfit_legs: 58\n\
             \x20\x20outfit_feet: 76\n\
             \x20\x20starting_skills:\n\
             \x20\x20\x20\x20- { skill_type: 0, value: 1, current_ticks: 0, max_ticks: 0 }\n\
             \x20\x20\x20\x20- { skill_type: 1, value: 220, current_ticks: 0, max_ticks: 0 }\n",
        )
        .unwrap();
        assert_eq!(cfg.session_ttl_days, 7);
        assert_eq!(cfg.auth_token_ttl_seconds, 60);
        assert_eq!(cfg.min_password_length, 8);
        assert_eq!(cfg.new_character.pos_x, 1028);
        assert_eq!(cfg.new_character.starting_skills.len(), 2);
    }

    #[test]
    fn rejects_a_config_missing_a_field() {
        assert!(SiteConfig::from_yaml("session_ttl_days: 7\n").is_err());
    }

    #[test]
    fn loads_the_checked_in_config_file() {
        let cfg = SiteConfig::load("config.yaml").unwrap();
        assert!(cfg.min_password_length >= 8);
    }

    #[test]
    fn loads_the_starting_character_template() {
        let cfg = SiteConfig::load("config.yaml").unwrap();
        assert_eq!(cfg.new_character.pos_x, 1028);
        assert_eq!(cfg.new_character.life, 150);
        assert_eq!(
            cfg.new_character.starting_skills.len(),
            2,
            "Level and Speed are the only skill types the game server understands"
        );
    }

    #[test]
    fn starting_skills_only_reference_skill_types_the_game_server_knows() {
        let cfg = SiteConfig::load("config.yaml").unwrap();
        for skill in &cfg.new_character.starting_skills {
            assert!(
                skill.skill_type == 0 || skill.skill_type == 1,
                "skill_type {} is not in the game server's SkillType enum (0 = Level, \
                 1 = Speed); seeding it would silently destroy the row on first logout",
                skill.skill_type
            );
        }
    }
}
