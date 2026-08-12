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
}

fn read_from_file() -> GameConfig {
    let contents =
        fs::read_to_string(&CONFIG.game_config_path).expect("failed to read game config");
    serde_yaml::from_str(&contents).expect("failed to parse game config")
}
