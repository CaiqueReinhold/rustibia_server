use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use tracing::warn;

use crate::entities::{
    agent::{Facing, OutfitColors, OutfitId, Pool},
    items::{Item, ItemConfig, ItemId},
    player::{InventorySlot, PlayerId},
    position::Position,
    skills::{SkillType, SkillValue},
};

#[derive(Error, Debug)]
pub enum PlayerRepositoryError {
    #[error("Player not found")]
    NotFound,
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct PlayerSnapshot {
    pub id: PlayerId,
    pub position: Position,
    pub origin: Position,
    pub facing: Facing,
    pub name: String,
    pub life: Pool,
    pub mana: Pool,
    pub capacity: Pool,
    pub outfit: (OutfitId, OutfitColors),
    pub skills: HashMap<SkillType, SkillValue>,
    pub inventory: HashMap<InventorySlot, Item>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct StoredItem {
    item_id: u16,
    amount: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<Vec<StoredItem>>,
}

pub struct PlayerRepository {
    pool: PgPool,
    items: Arc<HashMap<ItemId, Arc<ItemConfig>>>,
}

impl PlayerRepository {
    pub fn new(pool: PgPool, items: Arc<HashMap<ItemId, Arc<ItemConfig>>>) -> Self {
        Self { pool, items }
    }

    pub async fn get_by_id(&self, id: PlayerId) -> Result<PlayerSnapshot, PlayerRepositoryError> {
        use sqlx::Row;

        let row = sqlx::query(
            "SELECT id, name, pos_x, pos_y, pos_z, origin_x, origin_y, origin_z, \
             facing, life_cur, life_max, mana_cur, mana_max, cap_cur, cap_max, \
             outfit_id, outfit_head, outfit_body, outfit_legs, outfit_feet, inventory \
             FROM players WHERE id = $1",
        )
        .bind(id as i32)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(PlayerRepositoryError::NotFound)?;

        let skill_rows = sqlx::query(
            "SELECT skill_type, value, current_ticks, max_ticks \
             FROM player_skills WHERE player_id = $1",
        )
        .bind(id as i32)
        .fetch_all(&self.pool)
        .await?;

        let skills: HashMap<SkillType, SkillValue> = skill_rows
            .iter()
            .filter_map(|r| {
                let skill_type = i16_to_skill_type(r.try_get::<i16, _>("skill_type").ok()?)?;
                let value = SkillValue {
                    value: r.try_get::<i16, _>("value").ok()? as u16,
                    current_ticks: r.try_get::<i64, _>("current_ticks").ok()? as u64,
                    max_ticks: r.try_get::<i64, _>("max_ticks").ok()? as u64,
                };
                Some((skill_type, value))
            })
            .collect();

        let stored_inventory: sqlx::types::Json<HashMap<String, StoredItem>> =
            row.try_get("inventory")?;

        let inventory: HashMap<InventorySlot, Item> = stored_inventory
            .0
            .into_iter()
            .filter_map(|(slot_str, stored)| {
                let slot_id: u16 = slot_str.parse().ok()?;
                let slot = InventorySlot::from_id(slot_id)?;
                let item = self.restore_item(stored)?;
                Some((slot, item))
            })
            .collect();

        Ok(PlayerSnapshot {
            id: row.try_get::<i32, _>("id")? as u32,
            name: row.try_get("name")?,
            position: Position {
                x: row.try_get::<i32, _>("pos_x")? as u16,
                y: row.try_get::<i32, _>("pos_y")? as u16,
                z: row.try_get::<i16, _>("pos_z")? as u8,
            },
            origin: Position {
                x: row.try_get::<i32, _>("origin_x")? as u16,
                y: row.try_get::<i32, _>("origin_y")? as u16,
                z: row.try_get::<i16, _>("origin_z")? as u8,
            },
            facing: i16_to_facing(row.try_get::<i16, _>("facing")?)
                .ok_or(sqlx::Error::Decode("unknown facing discriminant".into()))?,
            life: Pool {
                current: row.try_get::<i32, _>("life_cur")? as u32,
                maximum: row.try_get::<i32, _>("life_max")? as u32,
            },
            mana: Pool {
                current: row.try_get::<i32, _>("mana_cur")? as u32,
                maximum: row.try_get::<i32, _>("mana_max")? as u32,
            },
            capacity: Pool {
                current: row.try_get::<i32, _>("cap_cur")? as u32,
                maximum: row.try_get::<i32, _>("cap_max")? as u32,
            },
            outfit: (
                row.try_get::<i16, _>("outfit_id")? as u16,
                (
                    row.try_get::<i16, _>("outfit_head")? as u8,
                    row.try_get::<i16, _>("outfit_body")? as u8,
                    row.try_get::<i16, _>("outfit_legs")? as u8,
                    row.try_get::<i16, _>("outfit_feet")? as u8,
                ),
            ),
            skills,
            inventory,
        })
    }

    pub async fn save(&self, snapshot: &PlayerSnapshot) -> Result<(), PlayerRepositoryError> {
        let inventory = serialize_inventory(&snapshot.inventory);
        let facing = facing_to_i16(snapshot.facing);
        let (outfit_id, (outfit_head, outfit_body, outfit_legs, outfit_feet)) = snapshot.outfit;

        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO players \
             (id, name, pos_x, pos_y, pos_z, origin_x, origin_y, origin_z, \
              facing, life_cur, life_max, mana_cur, mana_max, cap_cur, cap_max, \
              outfit_id, outfit_head, outfit_body, outfit_legs, outfit_feet, inventory) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21) \
             ON CONFLICT (id) DO UPDATE SET \
              name = EXCLUDED.name, \
              pos_x = EXCLUDED.pos_x, pos_y = EXCLUDED.pos_y, pos_z = EXCLUDED.pos_z, \
              origin_x = EXCLUDED.origin_x, origin_y = EXCLUDED.origin_y, origin_z = EXCLUDED.origin_z, \
              facing = EXCLUDED.facing, \
              life_cur = EXCLUDED.life_cur, life_max = EXCLUDED.life_max, \
              mana_cur = EXCLUDED.mana_cur, mana_max = EXCLUDED.mana_max, \
              cap_cur = EXCLUDED.cap_cur, cap_max = EXCLUDED.cap_max, \
              outfit_id = EXCLUDED.outfit_id, \
              outfit_head = EXCLUDED.outfit_head, outfit_body = EXCLUDED.outfit_body, \
              outfit_legs = EXCLUDED.outfit_legs, outfit_feet = EXCLUDED.outfit_feet, \
              inventory = EXCLUDED.inventory",
        )
        .bind(snapshot.id as i32)
        .bind(&snapshot.name)
        .bind(snapshot.position.x as i32)
        .bind(snapshot.position.y as i32)
        .bind(snapshot.position.z as i16)
        .bind(snapshot.origin.x as i32)
        .bind(snapshot.origin.y as i32)
        .bind(snapshot.origin.z as i16)
        .bind(facing)
        .bind(snapshot.life.current as i32)
        .bind(snapshot.life.maximum as i32)
        .bind(snapshot.mana.current as i32)
        .bind(snapshot.mana.maximum as i32)
        .bind(snapshot.capacity.current as i32)
        .bind(snapshot.capacity.maximum as i32)
        .bind(outfit_id as i16)
        .bind(outfit_head as i16)
        .bind(outfit_body as i16)
        .bind(outfit_legs as i16)
        .bind(outfit_feet as i16)
        .bind(sqlx::types::Json(&inventory))
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM player_skills WHERE player_id = $1")
            .bind(snapshot.id as i32)
            .execute(&mut *tx)
            .await?;

        for (skill_type, skill_value) in &snapshot.skills {
            sqlx::query(
                "INSERT INTO player_skills (player_id, skill_type, value, current_ticks, max_ticks) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(snapshot.id as i32)
            .bind(skill_type_to_i16(skill_type))
            .bind(skill_value.value as i16)
            .bind(skill_value.current_ticks as i64)
            .bind(skill_value.max_ticks as i64)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    fn restore_item(&self, stored: StoredItem) -> Option<Item> {
        let config = match self.items.get(&stored.item_id) {
            Some(c) => c.clone(),
            None => {
                warn!(
                    item_id = stored.item_id,
                    "skipping unknown item_id during inventory restore"
                );
                return None;
            }
        };
        let mut item = Item::new(stored.item_id, config, stored.amount);
        if let Some(children) = stored.content {
            let restored: Vec<Item> = children
                .into_iter()
                .filter_map(|c| self.restore_item(c))
                .collect();
            item.content = Some(restored);
        }
        Some(item)
    }
}

fn serialize_inventory(inventory: &HashMap<InventorySlot, Item>) -> HashMap<String, StoredItem> {
    inventory
        .iter()
        .map(|(slot, item)| (slot.as_id().to_string(), serialize_item(item)))
        .collect()
}

fn serialize_item(item: &Item) -> StoredItem {
    StoredItem {
        item_id: item.item_id,
        amount: item.amount,
        content: item
            .content
            .as_ref()
            .map(|children| children.iter().map(serialize_item).collect()),
    }
}

fn facing_to_i16(f: Facing) -> i16 {
    match f {
        Facing::North => 0,
        Facing::East => 1,
        Facing::South => 2,
        Facing::West => 3,
    }
}

fn i16_to_facing(n: i16) -> Option<Facing> {
    match n {
        0 => Some(Facing::North),
        1 => Some(Facing::East),
        2 => Some(Facing::South),
        3 => Some(Facing::West),
        _ => None,
    }
}

fn skill_type_to_i16(s: &SkillType) -> i16 {
    match s {
        SkillType::Level => 0,
        SkillType::Speed => 1,
    }
}

fn i16_to_skill_type(n: i16) -> Option<SkillType> {
    match n {
        0 => Some(SkillType::Level),
        1 => Some(SkillType::Speed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_item_omits_content_when_none() {
        let item = StoredItem {
            item_id: 2360,
            amount: 5,
            content: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(
            !json.contains("content"),
            "content field should be absent: {json}"
        );
    }

    #[test]
    fn stored_item_roundtrips_with_nested_content() {
        let item = StoredItem {
            item_id: 2148,
            amount: 1,
            content: Some(vec![StoredItem {
                item_id: 2360,
                amount: 10,
                content: None,
            }]),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: StoredItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.item_id, 2148);
        assert_eq!(back.amount, 1);
        let children = back.content.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].item_id, 2360);
        assert_eq!(children[0].amount, 10);
    }

    #[test]
    fn serialize_inventory_uses_slot_id_as_key() {
        use crate::entities::items::ItemConfig;
        use crate::entities::player::InventorySlot;
        use std::collections::HashSet;

        let config = Arc::new(ItemConfig::new(
            "sword".to_string(),
            None,
            None,
            HashSet::new(),
            HashSet::new(),
        ));
        let item = Item::new(2360, config, 1);
        let mut inv: HashMap<InventorySlot, Item> = HashMap::new();
        inv.insert(InventorySlot::RightHand, item);

        let stored = serialize_inventory(&inv);
        // RightHand::as_id() == 5
        assert!(
            stored.contains_key("5"),
            "expected key '5', got: {stored:?}"
        );
        assert_eq!(stored["5"].item_id, 2360);
        assert_eq!(stored["5"].amount, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_by_id_returns_not_found_for_missing_player(pool: PgPool) {
        let items = Arc::new(HashMap::new());
        let repo = PlayerRepository::new(pool, items);
        let result = repo.get_by_id(9999).await;
        assert!(
            matches!(result, Err(PlayerRepositoryError::NotFound)),
            "expected NotFound, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn save_and_get_by_id_roundtrip(pool: PgPool) {
        use crate::entities::agent::{Facing, Pool};
        use crate::entities::position::Position;

        let items = Arc::new(HashMap::new());
        let repo = PlayerRepository::new(pool, Arc::clone(&items));

        let snapshot = PlayerSnapshot {
            id: 1,
            name: "Rizael".to_string(),
            position: Position {
                x: 1028,
                y: 1028,
                z: 7,
            },
            origin: Position {
                x: 1028,
                y: 1028,
                z: 7,
            },
            facing: Facing::South,
            life: Pool {
                current: 100,
                maximum: 100,
            },
            mana: Pool {
                current: 100,
                maximum: 100,
            },
            capacity: Pool {
                current: 0,
                maximum: 40000,
            },
            outfit: (133, (1, 2, 3, 4)),
            skills: {
                let mut m = HashMap::new();
                m.insert(
                    SkillType::Level,
                    SkillValue {
                        value: 1,
                        current_ticks: 0,
                        max_ticks: 100,
                    },
                );
                m.insert(
                    SkillType::Speed,
                    SkillValue {
                        value: 120,
                        current_ticks: 0,
                        max_ticks: 0,
                    },
                );
                m
            },
            inventory: HashMap::new(),
        };

        repo.save(&snapshot).await.unwrap();

        let loaded = repo.get_by_id(1).await.unwrap();
        assert_eq!(loaded.name, "Rizael");
        assert_eq!(loaded.position.x, 1028);
        assert_eq!(loaded.position.z, 7);
        assert_eq!(loaded.facing, Facing::South);
        assert_eq!(loaded.life.current, 100);
        assert_eq!(loaded.life.maximum, 100);
        assert_eq!(loaded.capacity.maximum, 40000);
        assert_eq!(loaded.outfit, (133, (1, 2, 3, 4)));
        assert_eq!(loaded.skills[&SkillType::Level].value, 1);
        assert_eq!(loaded.skills[&SkillType::Speed].value, 120);
        assert!(loaded.inventory.is_empty());
        assert_eq!(loaded.origin.x, 1028);
        assert_eq!(loaded.origin.y, 1028);
        assert_eq!(loaded.origin.z, 7);
        assert_eq!(loaded.mana.current, 100);
        assert_eq!(loaded.mana.maximum, 100);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn save_overwrites_existing_player(pool: PgPool) {
        use crate::entities::agent::{Facing, Pool};
        use crate::entities::position::Position;

        let items = Arc::new(HashMap::new());
        let repo = PlayerRepository::new(pool, Arc::clone(&items));

        let mut snapshot = PlayerSnapshot {
            id: 1,
            name: "Rizael".to_string(),
            position: Position {
                x: 100,
                y: 100,
                z: 7,
            },
            origin: Position {
                x: 100,
                y: 100,
                z: 7,
            },
            facing: Facing::North,
            life: Pool {
                current: 80,
                maximum: 100,
            },
            mana: Pool {
                current: 50,
                maximum: 100,
            },
            capacity: Pool {
                current: 0,
                maximum: 40000,
            },
            outfit: (133, (0, 0, 0, 0)),
            skills: HashMap::new(),
            inventory: HashMap::new(),
        };

        repo.save(&snapshot).await.unwrap();

        snapshot.position = Position {
            x: 200,
            y: 300,
            z: 5,
        };
        snapshot.life.current = 60;
        repo.save(&snapshot).await.unwrap();

        let loaded = repo.get_by_id(1).await.unwrap();
        assert_eq!(loaded.position.x, 200);
        assert_eq!(loaded.position.y, 300);
        assert_eq!(loaded.position.z, 5);
        assert_eq!(loaded.life.current, 60);
    }
}
