use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use thiserror::Error;

use crate::entities::items::{FloorChangeDirection, ItemAttribute, ItemConfig, ItemFlag, ItemId};
use crate::entities::player::InventorySlot;

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
        _ => None,
    }
}

fn parse_inventory_slot(s: u64) -> Option<InventorySlot> {
    InventorySlot::from_id(s as u32)
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
                _ => return None,
            };
            Some(ItemAttribute::FloorChange(dir))
        }
        _ => {
            let n = value.as_u64()? as u32;
            match key {
                "capacity" => Some(ItemAttribute::Capacity(n as u8)),
                "weight" => Some(ItemAttribute::Weight(n)),
                "tile_friction" => Some(ItemAttribute::TileFriction(n)),
                _ => None,
            }
        }
    }
}

fn convert(raw: RawItemConfig) -> ItemConfig {
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

    ItemConfig::new(raw.name, raw.description, raw.article, flags, attributes)
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
        .map(|(id, raw)| (id, Arc::new(convert(raw))))
        .collect())
}
