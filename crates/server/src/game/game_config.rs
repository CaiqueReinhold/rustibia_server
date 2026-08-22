use std::fs;

use once_cell::sync::Lazy;
use serde::Deserialize;

use crate::entities::vocation::Vocation;
use crate::{
    config::CONFIG,
    entities::{chat::ChannelId, items::ItemId},
    game::Tick,
};

pub static GAME_CONFIG: Lazy<GameConfig> = Lazy::new(read_from_file);

#[derive(Deserialize)]
pub struct GameConfig {
    pub multi_action: MultiActionConfig,
    pub action: ItemActionConfig,
    pub movement: MovementConfig,
    pub chat: ChatConfig,
    pub effect_ids: EffectsConfig,
    pub text_colors: TextColors,
    pub combat: CombatConfig,
    pub skills: SkillsConfig,
}

#[derive(Deserialize)]
pub struct MovementConfig {
    pub wander_ticks: Tick,
}

#[derive(Deserialize)]
pub struct ItemActionConfig {
    pub use_item_cooldown_ticks: Tick,
}

#[derive(Deserialize)]
pub struct MultiActionConfig {
    #[serde(default)]
    pub diggable_ids: Vec<ItemId>,
    #[serde(default)]
    pub opened_hole_ids: Vec<ItemId>,
    #[serde(default)]
    pub rope_spot_ids: Vec<ItemId>,
}

#[derive(Deserialize)]
pub struct ChannelConfig {
    pub id: ChannelId,
    pub name: String,
}

#[derive(Deserialize)]
pub struct ChatConfig {
    pub server_channels: Vec<ChannelConfig>,
    pub max_message_length: usize,
    pub message_cooldown_ticks: Tick,
}

#[derive(Deserialize)]
pub struct EffectsConfig {
    pub life_hit: u16,
    pub shield_hit: u16,
    pub armor_hit: u16,
    pub poison_hit: u16,
    pub ice_hit: u16,
    pub earth_hit: u16,
    pub fire_hit: u16,
    pub energy_hit: u16,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Color(pub u8, pub u8, pub u8);

#[derive(Deserialize)]
pub struct TextColors {
    pub red: Color,
    pub lightgreen: Color,
    pub lightblue: Color,
    pub skyblue: Color,
    pub orange: Color,
    pub eletric_purple: Color,
}

#[derive(Deserialize)]
pub struct CombatConfig {
    pub auto_attack_ticks: Tick,
    pub pool_item_id: u16,
    pub unarmed_skill: u16,
}

#[derive(Deserialize)]
pub struct VocationMultipliers {
    pub melee: f32,
    pub distance: f32,
    pub magic: f32,
}

#[derive(Deserialize)]
pub struct SkillBases {
    pub melee: u64,
    pub distance: u64,
    pub magic: u64,
}

#[derive(Deserialize)]
pub struct VocationCurves {
    pub knight: VocationMultipliers,
    pub paladin: VocationMultipliers,
    pub sorcerer: VocationMultipliers,
    pub druid: VocationMultipliers,
}

impl VocationCurves {
    pub fn get(&self, vocation: Vocation) -> &VocationMultipliers {
        match vocation {
            Vocation::Knight => &self.knight,
            Vocation::Paladin => &self.paladin,
            Vocation::Sorcerer => &self.sorcerer,
            Vocation::Druid => &self.druid,
        }
    }
}

#[derive(Deserialize)]
pub struct SkillsConfig {
    pub min_level: u16,
    pub base: SkillBases,
    pub vocations: VocationCurves,
}

fn read_from_file() -> GameConfig {
    let contents =
        fs::read_to_string(&CONFIG.game_config_path).expect("failed to read game config");
    serde_yaml::from_str(&contents).expect("failed to parse game config")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the shape of the config file. A missing field here is a startup panic in
    /// production, because `read_from_file` expects rather than defaults.
    #[test]
    fn parses_the_chat_section() {
        let yaml = r#"
multi_action:
  rope_spot_ids: [386]
  opened_hole_ids: [21342]
  diggable_ids: [593]
action:
  use_item_cooldown_ticks: 20
movement:
  wander_ticks: 40
chat:
  server_channels:
    - id: 1
      name: World Chat
  max_message_length: 255
  message_cooldown_ticks: 10
"#;
        let config: GameConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.chat.max_message_length, 255);
        assert_eq!(config.chat.message_cooldown_ticks, 10);
        assert_eq!(config.chat.server_channels.len(), 1);
        assert_eq!(config.chat.server_channels[0].id, 1);
        assert_eq!(config.chat.server_channels[0].name, "World Chat");
    }

    /// The real file must satisfy the same shape.
    #[test]
    fn the_shipped_config_file_parses() {
        let contents = std::fs::read_to_string("assets/game_conf.yaml").unwrap();
        let config: GameConfig = serde_yaml::from_str(&contents).unwrap();
        assert!(config.chat.max_message_length > 0);
    }
}
