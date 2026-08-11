//! Database fixtures shared by the save tests (`player.rs`) and the login tests
//! (`login.rs`).
//!
//! They were private to `player.rs` until login moved out of it. Sharing them rather than
//! duplicating matters because of what they encode: `accounts.id` and `players.id` are
//! `GENERATED ALWAYS`, so ids cannot be chosen and must be read back, and `save` is an
//! `UPDATE` — a character row has to exist before anything here can write to it.

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::PgPool;

use crate::entities::{
    agent::{Facing, Pool},
    items::{ItemConfig, ItemId},
    position::Position,
    skills::{SkillType, SkillValue},
};
use crate::persistence::player::PlayerSnapshot;

/// An empty item catalogue. Restoring an inventory needs one, and every test here starts
/// a character with nothing equipped.
pub fn no_items() -> Arc<HashMap<ItemId, Arc<ItemConfig>>> {
    Arc::new(HashMap::new())
}

pub async fn insert_account(pool: &PgPool) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO accounts (email, password_hash) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("fixture-{}@example.com", uuid::Uuid::now_v7()))
    .bind("not-a-real-hash")
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Creates the character row. The game server can no longer do this itself — the website
/// owns character creation, and `save` only updates.
pub async fn insert_character(pool: &PgPool, account_id: i32) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO players \
         (account_id, name, vocation, sex, pos_x, pos_y, pos_z, origin_x, origin_y, origin_z, \
          facing, life_cur, life_max, mana_cur, mana_max, cap_cur, cap_max, \
          outfit_id, outfit_head, outfit_body, outfit_legs, outfit_feet) \
         VALUES ($1, $2, 0, 1, 1028, 1028, 7, 1028, 1028, 7, \
                 2, 150, 150, 0, 0, 400, 400, 133, 1, 2, 3, 4) \
         RETURNING id",
    )
    .bind(account_id)
    .bind(format!("Rizael{}", uuid::Uuid::now_v7().as_u128() % 100000))
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A live game token for `character_id`, as the site would have minted.
pub async fn insert_token(pool: &PgPool, character_id: i32) -> String {
    insert_token_valid_for(pool, character_id, "1 hour").await
}

/// `interval` is Postgres interval syntax, and may be negative (`"-1 hour"`) for an
/// already-expired token.
///
/// Stores the digest and returns the plaintext, exactly as the site's mint path does —
/// inserting the plaintext here would make the load tests pass against a lookup that
/// production could never satisfy.
pub async fn insert_token_valid_for(pool: &PgPool, character_id: i32, interval: &str) -> String {
    let token = format!("token-{}", uuid::Uuid::now_v7());
    sqlx::query(&format!(
        "INSERT INTO game_tokens (token_hash, character_id, valid_until) \
         VALUES ($1, $2, NOW() + INTERVAL '{interval}')"
    ))
    .bind(super::login::hash_token_for_tests(&token))
    .bind(character_id)
    .execute(pool)
    .await
    .unwrap();
    token
}

pub async fn token_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM game_tokens")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The snapshot used by every save test. `id` and `account_id` are parameters because
/// both come from identity sequences rather than literals.
pub fn a_test_snapshot(id: u32, account_id: i32) -> PlayerSnapshot {
    PlayerSnapshot {
        id,
        account_id,
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
        skills: HashMap::from([
            (
                SkillType::Level,
                SkillValue {
                    value: 1,
                    current_ticks: 0,
                    max_ticks: 100,
                },
            ),
            (
                SkillType::Speed,
                SkillValue {
                    value: 120,
                    current_ticks: 0,
                    max_ticks: 0,
                },
            ),
        ]),
        inventory: HashMap::new(),
    }
}
