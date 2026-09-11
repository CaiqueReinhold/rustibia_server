use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use once_cell::sync::Lazy;
use serde::Deserialize;
use thiserror::Error;

use crate::config::CONFIG;
use crate::entities::Bounds;
use crate::entities::combat::{AmmoType, CombatElement, WeaponType};
use crate::entities::effects::MissileId;
use crate::entities::inventory::InventorySlot;
use crate::entities::items::{
    FloorChangeDirection, ItemAction, ItemAttribute, ItemConfig, ItemFlag, ItemId, ItemMultiAction,
};
use crate::game::TickDelta;

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
        "multiuse" => Some(ItemFlag::Multiuse),
        "avoid" => Some(ItemFlag::Avoid),
        "ammo_container" => Some(ItemFlag::AmmoContainer),
        "liquidpool" => Some(ItemFlag::LiquidPool),
        _ => None,
    }
}

fn parse_inventory_slot(s: u64) -> Option<InventorySlot> {
    InventorySlot::from_id(s)
}

/// A `{min, max}` mapping. Both halves are required: a range missing one of them
/// is corrupt data, not a range with a default, and rolling it would invent an
/// amount rather than refuse one.
fn parse_bounds(value: &serde_yaml::Value) -> Option<Bounds> {
    let min = value.get("min")?.as_u64()? as u32;
    let max = value.get("max")?.as_u64()? as u32;
    if min > max {
        return None;
    }
    Some(Bounds { min, max })
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
                    let item_id = params.parse::<u16>().map(ItemId).ok()?;
                    ItemAction::Transform { into: item_id }
                }
                _ => return None,
            };
            Some(ItemAttribute::Action(action))
        }
        "potion" => {
            let health = match value.get("health") {
                Some(bounds) => Some(parse_bounds(bounds)?),
                None => None,
            };
            let mana = match value.get("mana") {
                Some(bounds) => Some(parse_bounds(bounds)?),
                None => None,
            };
            let flask = match value.get("flask") {
                Some(flask) => Some(ItemId(u16::try_from(flask.as_u64()?).ok()?)),
                None => None,
            };
            if health.is_none() && mana.is_none() {
                return None;
            }
            Some(ItemAttribute::MultiAction(ItemMultiAction::Potion {
                health,
                mana,
                flask,
            }))
        }
        "decay" => {
            let duration = TickDelta(value.get("duration")?.as_u64()?);
            let decay_to = ItemId(value.get("decay_to")?.as_u64()? as u16);
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
            "distance" => Some(ItemAttribute::WeaponType(WeaponType::Distance)),
            "wand" => Some(ItemAttribute::WeaponType(WeaponType::Wand)),
            "rod" => Some(ItemAttribute::WeaponType(WeaponType::Rod)),
            _ => None,
        },
        "ammo_type" => match value.as_str()? {
            "arrow" => Some(ItemAttribute::AmmoType(AmmoType::Arrow)),
            "bolt" => Some(ItemAttribute::AmmoType(AmmoType::Bolt)),
            _ => None,
        },
        "extra_defense" => Some(ItemAttribute::ExtraDef(value.as_i64()? as i16)),
        "range" => Some(ItemAttribute::WeaponRange(value.as_i64()? as u8)),
        "hit_chance" => Some(ItemAttribute::HitChance(value.as_i64()? as i16)),
        "max_hit_chance" => Some(ItemAttribute::MaxHitChance(value.as_i64()? as u8)),
        "mana_cost" => Some(ItemAttribute::ManaCost(value.as_i64()? as u32)),
        "missile_id" => Some(ItemAttribute::MissileId(MissileId(value.as_i64()? as u16))),
        _ => {
            let n = value.as_u64()? as u32;
            match key {
                "capacity" => Some(ItemAttribute::Capacity(n as u8)),
                "weight" => Some(ItemAttribute::Weight(n)),
                "tile_friction" => Some(ItemAttribute::TileFriction(n as u16)),
                "armor" => Some(ItemAttribute::Armor(n as u16)),
                "defense" => Some(ItemAttribute::Defense(n as u16)),
                _ => None,
            }
        }
    }
}

fn convert(id: ItemId, raw: RawItemConfig) -> ItemConfig {
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
        raw.flags.iter().filter_map(|s| parse_flag(s)),
        attributes,
    )
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn load_items(
    path: impl AsRef<Path>,
) -> Result<HashMap<ItemId, Arc<ItemConfig>>, ItemsLoadError> {
    load_items_from_str(&fs::read_to_string(path)?)
}

/// Split out from `load_items` so the whole read path -- deserialize, `convert`,
/// `parse_attribute`, the `filter_map` that drops what it cannot read -- can be
/// exercised over a document the caller owns rather than over the shipped file.
fn load_items_from_str(contents: &str) -> Result<HashMap<ItemId, Arc<ItemConfig>>, ItemsLoadError> {
    let file: ItemsFile = serde_yaml::from_str(contents)?;
    Ok(file
        .items
        .into_iter()
        .map(|(id, raw)| (id, Arc::new(convert(id, raw))))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::items::ItemId;

    fn parse(key: &str, value: &str) -> Option<ItemAttribute> {
        parse_attribute(key, &serde_yaml::from_str(value).unwrap())
    }

    #[test]
    fn armour_and_defence_reach_the_config() {
        assert_eq!(parse("armor", "11"), Some(ItemAttribute::Armor(11)));
        assert_eq!(parse("defense", "25"), Some(ItemAttribute::Defense(25)));
    }

    /// `extradef` runs -3..3 in the source data. The `_ =>` fallback reads through
    /// `as_u64()`, which returns `None` for a negative and would drop these silently —
    /// the reading-side twin of the generators dropping fields on the way out.
    #[test]
    fn a_negative_extra_defence_is_not_dropped() {
        assert_eq!(
            parse("extra_defense", "-2"),
            Some(ItemAttribute::ExtraDef(-2))
        );
        assert_eq!(
            parse("extra_defense", "3"),
            Some(ItemAttribute::ExtraDef(3))
        );
    }

    /// The catalogue is the real check: a key that parses in isolation but is spelled
    /// differently in `items.yaml` reaches nothing.
    #[test]
    fn the_shipped_catalogue_carries_armour_and_defence() {
        let items = load_items(&CONFIG.items_file_path).unwrap();
        let armoured = items.values().filter(|c| c.attr_armor().is_some()).count();
        let defended = items
            .values()
            .filter(|c| c.attr_defense().is_some())
            .count();
        let extra = items
            .values()
            .filter(|c| c.attr_extra_def().is_some_and(|d| d < 0))
            .count();

        assert!(armoured > 300, "only {armoured} items carry armour");
        assert!(defended > 500, "only {defended} items carry defence");
        assert!(extra > 0, "no item kept a negative extra defence");
    }

    /// `devileye` carries `hit_chance: -20`, so this key has the same reason to sit above
    /// the unsigned fallback that `extra_defense` does.
    #[test]
    fn a_negative_hit_chance_is_not_dropped() {
        assert_eq!(
            parse("hit_chance", "-20"),
            Some(ItemAttribute::HitChance(-20))
        );
        assert_eq!(parse("hit_chance", "7"), Some(ItemAttribute::HitChance(7)));
        assert_eq!(
            parse("max_hit_chance", "91"),
            Some(ItemAttribute::MaxHitChance(91))
        );
    }

    /// Both keys were generated into `items.yaml` from the start and read by nothing until
    /// the distance hit roll existed — exactly the shape `the-asset-generators-drop-fields-
    /// silently` warns about, in the other direction.
    #[test]
    fn the_shipped_catalogue_carries_both_hit_chances() {
        let items = load_items(&CONFIG.items_file_path).unwrap();
        let bonuses = items
            .values()
            .filter(|c| c.attr_hit_chance().is_some())
            .count();
        let ceilings = items
            .values()
            .filter(|c| c.attr_max_hit_chance().is_some())
            .count();
        let penalties = items
            .values()
            .filter(|c| c.attr_hit_chance().is_some_and(|h| h < 0))
            .count();

        assert!(bonuses > 40, "only {bonuses} items carry a hit chance");
        assert!(ceilings > 30, "only {ceilings} items carry a ceiling");
        assert!(penalties > 0, "no item kept a negative hit chance");
    }

    fn bounds(min: u32, max: u32) -> Option<Bounds> {
        Some(Bounds { min, max })
    }

    fn potion(value: &str) -> Option<ItemAttribute> {
        parse("potion", value)
    }

    #[test]
    fn a_potion_carries_only_the_pools_it_restores() {
        assert_eq!(
            potion("health:\n  min: 125\n  max: 175"),
            Some(ItemAttribute::MultiAction(ItemMultiAction::Potion {
                health: bounds(125, 175),
                mana: None,
                flask: None,
            }))
        );
        assert_eq!(
            potion("mana:\n  min: 75\n  max: 125"),
            Some(ItemAttribute::MultiAction(ItemMultiAction::Potion {
                health: None,
                mana: bounds(75, 125),
                flask: None,
            }))
        );
        assert_eq!(
            potion("health:\n  min: 420\n  max: 580\nmana:\n  min: 180\n  max: 220"),
            Some(ItemAttribute::MultiAction(ItemMultiAction::Potion {
                health: bounds(420, 580),
                mana: bounds(180, 220),
                flask: None,
            }))
        );
    }

    /// The half-parsed potion is the dangerous one. `filter_map` in `convert`
    /// drops whatever this returns `None` for, so a spirit potion that kept only
    /// its readable half would restore mana and no life, in game, with nothing
    /// logged -- worse than an item that refuses to work.
    #[test]
    fn a_pool_that_does_not_parse_takes_the_whole_potion_with_it() {
        assert_eq!(potion("health:\n  min: 125"), None, "no max");
        assert_eq!(potion("health:\n  max: 175"), None, "no min");
        assert_eq!(
            potion("health:\n  min: 175\n  max: 125"),
            None,
            "min above max"
        );
        assert_eq!(
            potion("health:\n  min: 250\n  max: 350\nmana:\n  min: 100"),
            None,
            "the second pool is the broken one"
        );
        assert_eq!(
            potion("health: 125"),
            None,
            "a scalar where a range belongs"
        );
    }

    /// A potion that restores nothing is a data error, not a potion. Accepting it
    /// would put an item in the game that spends a charge for no effect.
    #[test]
    fn a_potion_with_neither_pool_is_refused() {
        assert_eq!(potion("{}"), None);
        assert_eq!(potion("level: 80"), None, "gates alone are not an effect");
    }

    /// The unit tests above prove `parse_attribute` alone. This proves the rest of
    /// the read path: that `convert`'s `filter_map` keeps the attribute rather than
    /// dropping it, and that `attr_multi_action` finds it again on the far side.
    /// The document is the test's own -- what `items.yaml` happens to carry is
    /// config, and pinning it here would make it a constant.
    #[test]
    fn a_potion_survives_the_whole_load_path() {
        let items = load_items_from_str(
            "
items:
  1:
    name: a health potion
    flags: [usable, multiuse]
    attributes:
      weight: 270
      potion:
        health:
          min: 125
          max: 175
        flask: 284
  2:
    name: a spirit potion
    attributes:
      potion:
        health:
          min: 250
          max: 350
        mana:
          min: 100
          max: 200
        flask: 284
  3:
    name: a broken potion
    attributes:
      potion:
        health:
          min: 125
",
        )
        .unwrap();

        assert_eq!(
            items[&ItemId(1)].attr_multi_action(),
            Some(ItemMultiAction::Potion {
                health: Some(Bounds { min: 125, max: 175 }),
                mana: None,
                flask: Some(ItemId(284)),
            })
        );
        assert_eq!(
            items[&ItemId(2)].attr_multi_action(),
            Some(ItemMultiAction::Potion {
                health: Some(Bounds { min: 250, max: 350 }),
                mana: Some(Bounds { min: 100, max: 200 }),
                flask: Some(ItemId(284)),
            })
        );
        // Dropped, and the item still loads -- which is exactly why the emitter has
        // a gate of its own: nothing here can tell you the potion went missing.
        assert_eq!(items[&ItemId(3)].attr_multi_action(), None);
        assert_eq!(items[&ItemId(3)].name, "a broken potion");
    }

    #[test]
    fn a_potion_carries_the_flask_it_leaves_behind() {
        assert_eq!(
            potion("health:\n  min: 125\n  max: 175\nflask: 284"),
            Some(ItemAttribute::MultiAction(ItemMultiAction::Potion {
                health: bounds(125, 175),
                mana: None,
                flask: Some(ItemId(284)),
            }))
        );
    }

    #[test]
    fn a_flask_that_is_not_an_item_id_takes_the_whole_potion_with_it() {
        assert_eq!(potion("health:\n  min: 1\n  max: 2\nflask: 99999"), None);
        assert_eq!(potion("health:\n  min: 1\n  max: 2\nflask: nope"), None);
    }
}
