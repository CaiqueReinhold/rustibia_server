use std::fs;

use once_cell::sync::Lazy;
use serde::Deserialize;

use crate::entities::vocation::Vocation;
use crate::{
    config::CONFIG,
    entities::{
        chat::ChannelId,
        effects::EffectId,
        items::{ItemId, ItemMultiAction},
    },
    game::TickDelta,
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
    pub wander_ticks: TickDelta,
    pub wander_distance: u16,
}

#[derive(Deserialize)]
pub struct ItemActionConfig {
    pub use_item_cooldown_ticks: TickDelta,
}

#[derive(Deserialize)]
pub struct MultiActionConfig {
    #[serde(default)]
    pub shovel_ids: Vec<ItemId>,
    #[serde(default)]
    pub rope_ids: Vec<ItemId>,
    #[serde(default)]
    pub diggable_ids: Vec<ItemId>,
    #[serde(default)]
    pub opened_hole_ids: Vec<ItemId>,
    #[serde(default)]
    pub rope_spot_ids: Vec<ItemId>,
}

impl MultiActionConfig {
    pub fn tool_action(&self, item_id: ItemId) -> Option<ItemMultiAction> {
        if self.shovel_ids.contains(&item_id) {
            Some(ItemMultiAction::Shovel)
        } else if self.rope_ids.contains(&item_id) {
            Some(ItemMultiAction::Rope)
        } else {
            None
        }
    }
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
    pub message_cooldown_ticks: TickDelta,
}

#[derive(Deserialize)]
pub struct EffectsConfig {
    pub life_hit: EffectId,
    pub shield_hit: EffectId,
    pub armor_hit: EffectId,
    pub poison_hit: EffectId,
    pub ice_hit: EffectId,
    pub earth_hit: EffectId,
    pub fire_hit: EffectId,
    pub energy_hit: EffectId,
    pub holy_hit: EffectId,
    pub death_hit: EffectId,
    pub potion_use: EffectId,
    pub miss: EffectId,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Color(pub u8, pub u8, pub u8);

#[derive(Deserialize)]
pub struct TextColors {
    pub white: Color,
    pub red: Color,
    pub lightgreen: Color,
    pub lightblue: Color,
    pub skyblue: Color,
    pub orange: Color,
    pub eletric_purple: Color,
}

#[derive(Deserialize)]
pub struct CombatConfig {
    pub auto_attack_ticks: TickDelta,
    pub pool_item_id: ItemId,
    pub human_corpse_item_id: ItemId,
    pub unarmed_skill: u16,
}

#[derive(Deserialize)]
pub struct VocationMultipliers {
    pub melee: f32,
    pub distance: f32,
    pub magic: f32,
    pub shielding: f32,
}

#[derive(Deserialize)]
pub struct SkillBases {
    pub melee: u64,
    pub distance: u64,
    pub magic: u64,
    pub shielding: u64,
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
