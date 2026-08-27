use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use once_cell::sync::Lazy;
use serde::Deserialize;
use thiserror::Error;

use crate::config::CONFIG;
use crate::entities::combat::{AmmoType, CombatElement, WeaponType};
use crate::entities::items::{
    FloorChangeDirection, ItemAction, ItemAttribute, ItemConfig, ItemFlag, ItemId, ItemMultiAction,
};
use crate::entities::player::InventorySlot;
use crate::game::Tick;

/// The item catalogue, loaded once from `assets/items.yaml`. Immutable after load and
/// read by every subsystem, so it is a global for the same reason `GAME_CONFIG` is.
pub static ITEM_CONFIGS: Lazy<Arc<HashMap<ItemId, Arc<ItemConfig>>>> = Lazy::new(|| {
    Arc::new(load_items(&CONFIG.items_file_path).expect("failed to load item configs"))
});

#[derive(Error, Debug)]
pub enum ItemsLoadError {
    #[error("I/O error: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    ParseError(#[from] serde_yaml::Error),
}

// ── Raw YAML deserialization types ────────────────────────────────────────────

#[derive(Deserialize)]
struct RawItemConfig {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    article: Option<String>,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    attributes: HashMap<String, serde_yaml::Value>,
}

#[derive(Deserialize)]
struct ItemsFile {
    items: HashMap<ItemId, RawItemConfig>,
}

// ── Conversion ────────────────────────────────────────────────────────────────

fn parse_flag(s: &str) -> Option<ItemFlag> {
    match s {
        "ground" => Some(ItemFlag::Ground),
        "unmove" => Some(ItemFlag::Unmove),
        "unpass" => Some(ItemFlag::Unpass),
        "take" => Some(ItemFlag::Take),
        "cumulative" => Some(ItemFlag::Cumulative),
        "fullbank" => Some(ItemFlag::FullBank),
        "bottom" => Some(ItemFlag::Bottom),
        "container" => Some(ItemFlag::Container),
        "usable" => Some(ItemFlag::Usable),
        "avoid" => Some(ItemFlag::Avoid),
        "ammo_container" => Some(ItemFlag::AmmoContainer),
        "liquidpool" => Some(ItemFlag::LiquidPool),
        _ => None,
    }
}

fn parse_inventory_slot(s: u64) -> Option<InventorySlot> {
    InventorySlot::from_id(s as u16)
}

fn parse_attribute(key: &str, value: &serde_yaml::Value) -> Option<ItemAttribute> {
    match key {
        "slot" => {
            let slot = parse_inventory_slot(value.as_u64()?)?;
            Some(ItemAttribute::Inventory(slot))
        }
        "floor_change" => {
            let dir = match value.as_str()? {
                "up" => FloorChangeDirection::Up,
                "down" => FloorChangeDirection::Down,
                "north" => FloorChangeDirection::North,
                "east" => FloorChangeDirection::East,
                "south" => FloorChangeDirection::South,
                "west" => FloorChangeDirection::West,
                _ => return None,
            };
            Some(ItemAttribute::FloorChange(dir))
        }
        "action" => {
            let mut iter = value.as_str()?.split("(");
            let action_name = iter.next()?;
            let params = iter.next()?.trim_end_matches(')');
            let action = match action_name {
                "transform" => {
                    let item_id = params.parse::<u16>().ok()?;
                    ItemAction::Transform { into: item_id }
                }
                _ => return None,
            };
            Some(ItemAttribute::Action(action))
        }
        "multi_action" => match value.as_str()? {
            "shovel" => Some(ItemAttribute::MultiAction(ItemMultiAction::Shovel)),
            "rope" => Some(ItemAttribute::MultiAction(ItemMultiAction::Rope)),
            _ => None,
        },
        "decay" => {
            let duration = value.get("duration")?.as_u64()? as Tick;
            let decay_to = value.get("decay_to")?.as_u64()? as ItemId;
            Some(ItemAttribute::Decay { duration, decay_to })
        }
        "attack" => Some(ItemAttribute::WeaponAttack(value.as_i64()? as u16)),
        "element" => match value.as_str()? {
            "ice" => Some(ItemAttribute::WeaponElement(CombatElement::Ice)),
            "physical" => Some(ItemAttribute::WeaponElement(CombatElement::Physical)),
            "fire" => Some(ItemAttribute::WeaponElement(CombatElement::Fire)),
            "earth" => Some(ItemAttribute::WeaponElement(CombatElement::Earth)),
            "energy" => Some(ItemAttribute::WeaponElement(CombatElement::Energy)),
            _ => None,
        },
        "weapon_type" => match value.as_str()? {
            "axe" => Some(ItemAttribute::WeaponType(WeaponType::Axe)),
            "sword" => Some(ItemAttribute::WeaponType(WeaponType::Sword)),
            "club" => Some(ItemAttribute::WeaponType(WeaponType::Club)),
            "bow" => Some(ItemAttribute::WeaponType(WeaponType::Bow)),
            "crossbow" => Some(ItemAttribute::WeaponType(WeaponType::Crossbow)),
            "wand" => Some(ItemAttribute::WeaponType(WeaponType::Wand)),
            "rod" => Some(ItemAttribute::WeaponType(WeaponType::Rod)),
            _ => None,
        },
        "ammo_type" => match value.as_str()? {
            "arrow" => Some(ItemAttribute::AmmoType(AmmoType::Arrow)),
            "bolt" => Some(ItemAttribute::AmmoType(AmmoType::Bolt)),
            _ => None,
        },
        "range" => Some(ItemAttribute::WeaponRange(value.as_i64()? as u8)),
        "mana_cost" => Some(ItemAttribute::ManaCost(value.as_i64()? as u32)),
        "missile_id" => Some(ItemAttribute::MissileId(value.as_i64()? as u16)),
        _ => {
            let n = value.as_u64()? as u32;
            match key {
                "capacity" => Some(ItemAttribute::Capacity(n as u8)),
                "weight" => Some(ItemAttribute::Weight(n)),
                "tile_friction" => Some(ItemAttribute::TileFriction(n as u16)),
                _ => None,
            }
        }
    }
}

fn convert(id: u16, raw: RawItemConfig) -> ItemConfig {
    let flags = raw
        .flags
        .iter()
        .filter_map(|s| parse_flag(s))
        .collect::<HashSet<_>>();

    let attributes = raw
        .attributes
        .iter()
        .filter_map(|(k, v)| parse_attribute(k, v))
        .collect::<HashSet<_>>();

    ItemConfig::new(
        id,
        raw.name,
        raw.description,
        raw.article,
        flags,
        attributes,
    )
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn load_items(
    path: impl AsRef<Path>,
) -> Result<HashMap<ItemId, Arc<ItemConfig>>, ItemsLoadError> {
    let contents = fs::read_to_string(path)?;
    let file: ItemsFile = serde_yaml::from_str(&contents)?;
    Ok(file
        .items
        .into_iter()
        .map(|(id, raw)| (id, Arc::new(convert(id, raw))))
        .collect())
}
