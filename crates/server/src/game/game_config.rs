use std::fs;

use once_cell::sync::Lazy;
use serde::Deserialize;

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
    /// Characters, not bytes — the client caps its input field by character count, and
    /// measuring the same thing on both sides means no composable message is refused.
    ///
    /// This still protects the `SRV_CHAT_MESSAGE` encoder, which writes `len() as u16`:
    /// an unbounded message would truncate that prefix and corrupt the stream, and the
    /// worst case here is 4 bytes per character, so the guard only lapses above ~16383.
    pub max_message_length: usize,
    pub message_cooldown_ticks: Tick,
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
