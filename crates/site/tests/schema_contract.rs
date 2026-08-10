//! Asserts the database shape that `game_server` depends on.
//!
//! `game_server` keeps its own row structs and its own SQL; nothing at compile time
//! connects the two repositories. These tests are that connection. If one fails, a
//! schema change here is about to break the game server at runtime.
//!
//! Sources in `game_server` this mirrors:
//!   - `src/persistence/player.rs`  — players + player_skills columns
//!   - `src/persistence/auth.rs`    — auth_tokens columns

use sqlx::{PgPool, Row};

async fn column_type(pool: &PgPool, table: &str, column: &str) -> Option<String> {
    sqlx::query(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2",
    )
    .bind(table)
    .bind(column)
    .fetch_optional(pool)
    .await
    .unwrap()
    .map(|row| row.get::<String, _>("data_type"))
}

async fn assert_column(pool: &PgPool, table: &str, column: &str, expected: &str) {
    let actual = column_type(pool, table, column).await;
    assert_eq!(
        actual.as_deref(),
        Some(expected),
        "game_server reads {table}.{column} as {expected}; found {actual:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn players_has_every_column_the_game_server_reads(pool: PgPool) {
    for column in [
        "pos_x", "pos_y", "origin_x", "origin_y", "life_cur", "life_max", "mana_cur", "mana_max",
        "cap_cur", "cap_max",
    ] {
        assert_column(&pool, "players", column, "integer").await;
    }
    for column in [
        "pos_z", "origin_z", "facing", "outfit_id", "outfit_head", "outfit_body", "outfit_legs",
        "outfit_feet",
    ] {
        assert_column(&pool, "players", column, "smallint").await;
    }
    assert_column(&pool, "players", "id", "integer").await;
    assert_column(&pool, "players", "account_id", "integer").await;
    assert_column(&pool, "players", "name", "text").await;
    assert_column(&pool, "players", "inventory", "jsonb").await;
}

#[sqlx::test(migrations = "./migrations")]
async fn players_has_the_soft_delete_column_the_server_filters_on(pool: PgPool) {
    assert_column(&pool, "players", "deleted_at", "timestamp with time zone").await;
}

#[sqlx::test(migrations = "./migrations")]
async fn player_skills_has_every_column_the_game_server_reads(pool: PgPool) {
    assert_column(&pool, "player_skills", "player_id", "integer").await;
    assert_column(&pool, "player_skills", "skill_type", "smallint").await;
    assert_column(&pool, "player_skills", "value", "smallint").await;
    assert_column(&pool, "player_skills", "current_ticks", "bigint").await;
    assert_column(&pool, "player_skills", "max_ticks", "bigint").await;
}

#[sqlx::test(migrations = "./migrations")]
async fn auth_tokens_has_every_column_the_game_server_reads(pool: PgPool) {
    assert_column(&pool, "auth_tokens", "token", "text").await;
    assert_column(&pool, "auth_tokens", "account_id", "integer").await;
    assert_column(&pool, "auth_tokens", "valid_until", "timestamp with time zone").await;
}

#[sqlx::test(migrations = "./migrations")]
async fn online_players_has_the_columns_the_game_server_writes(pool: PgPool) {
    assert_column(&pool, "online_players", "character_id", "integer").await;
    assert_column(&pool, "online_players", "since", "timestamp with time zone").await;
}

#[sqlx::test(migrations = "./migrations")]
async fn the_game_servers_player_select_still_executes(pool: PgPool) {
    // Verbatim from game_server/src/persistence/player.rs:65-69, plus the
    // `deleted_at IS NULL` clause added by task 17 of this plan.
    let result = sqlx::query(
        "SELECT id, account_id, name, pos_x, pos_y, pos_z, origin_x, origin_y, origin_z, \
         facing, life_cur, life_max, mana_cur, mana_max, cap_cur, cap_max, \
         outfit_id, outfit_head, outfit_body, outfit_legs, outfit_feet, inventory \
         FROM players WHERE id = $1 AND account_id = $2 AND deleted_at IS NULL",
    )
    .bind(1i32)
    .bind(1i32)
    .fetch_optional(&pool)
    .await;

    assert!(result.is_ok(), "the game server's SELECT must still be valid: {result:?}");
}

#[sqlx::test(migrations = "./migrations")]
async fn the_game_servers_auth_token_select_still_executes(pool: PgPool) {
    // Verbatim from game_server/src/persistence/auth.rs:29-32.
    let result = sqlx::query(
        "SELECT account_id FROM auth_tokens WHERE token = $1 AND valid_until > NOW()",
    )
    .bind("anything")
    .fetch_optional(&pool)
    .await;

    assert!(result.is_ok(), "the game server's token lookup must still be valid: {result:?}");
}
