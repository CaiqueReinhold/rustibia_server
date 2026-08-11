//! Asserts the database shape that `crates/server` still depends on.
//!
//! Shrinking, deliberately. Login now goes through `POST /internal/game-tokens/redeem`, so
//! the `game_tokens` columns and the token lookup are asserted by the site's own code and
//! by `rustibia-contract`'s types — a compile error there beats a runtime assertion here.
//! What remains is what the game server continues to reach for directly:
//!
//!   - `crates/server/src/persistence/player.rs` — `save`: players + player_skills
//!   - `crates/server/src/persistence/online.rs` — online_players
//!   - `crates/server/src/persistence/login.rs`  — `SqlLoginRepository`, the rollback path
//!
//! These go away when saving and online tracking move to REST as well.

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
        "pos_z",
        "origin_z",
        "facing",
        "outfit_id",
        "outfit_head",
        "outfit_body",
        "outfit_legs",
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
async fn online_players_has_the_columns_the_game_server_writes(pool: PgPool) {
    assert_column(&pool, "online_players", "character_id", "integer").await;
    assert_column(&pool, "online_players", "since", "timestamp with time zone").await;
}

#[sqlx::test(migrations = "./migrations")]
async fn the_game_servers_player_select_still_executes(pool: PgPool) {
    // Verbatim from `SqlLoginRepository::redeem_inner` in
    // crates/server/src/persistence/login.rs. The site runs the same SELECT in
    // `db::login::redeem`; this asserts the game server's rollback path stays valid too.
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

    assert!(
        result.is_ok(),
        "the game server's SELECT must still be valid: {result:?}"
    );
}

/// The columns the redemption path binds and reads. `character_id` is the point of the
/// table: it is what removes the client from the decision about which character loads.
#[sqlx::test(migrations = "./migrations")]
async fn game_tokens_has_the_columns_the_login_path_needs(pool: PgPool) {
    assert_column(&pool, "game_tokens", "token_hash", "text").await;
    assert_column(&pool, "game_tokens", "character_id", "integer").await;
    assert_column(
        &pool,
        "game_tokens",
        "valid_until",
        "timestamp with time zone",
    )
    .await;

    assert_eq!(
        column_type(&pool, "game_tokens", "account_id").await,
        None,
        "account_id must be gone; ownership is decided at mint time, not redemption"
    );
    assert_eq!(
        column_type(&pool, "auth_tokens", "token_hash").await,
        None,
        "auth_tokens must be gone, not left behind alongside its replacement"
    );
}

/// Both `db::login::redeem` here and `SqlLoginRepository` in the game server spend a
/// token with this statement. It is the one place single-use redemption is implemented,
/// so it earns an assertion that it still compiles against the schema.
#[sqlx::test(migrations = "./migrations")]
async fn the_single_use_token_delete_still_executes(pool: PgPool) {
    let result = sqlx::query(
        "DELETE FROM game_tokens WHERE token_hash = $1 AND valid_until > NOW() \
         RETURNING character_id",
    )
    .bind("anything")
    .fetch_optional(&pool)
    .await;

    assert!(
        result.is_ok(),
        "the redemption statement must still be valid: {result:?}"
    );
}

/// Supersede-prior is enforced by the schema, not by remembering to write a DELETE.
/// A second mint path added later must be unable to leave two live tickets behind.
#[sqlx::test(migrations = "./migrations")]
async fn a_character_cannot_hold_two_tokens(pool: PgPool) {
    let account_id: i32 = sqlx::query_scalar(
        "INSERT INTO accounts (email, password_hash) VALUES ('a@example.com', 'x') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let character_id: i32 = sqlx::query_scalar(
        "INSERT INTO players \
         (account_id, name, vocation, sex, pos_x, pos_y, pos_z, origin_x, origin_y, origin_z, \
          facing, life_cur, life_max, mana_cur, mana_max, cap_cur, cap_max, \
          outfit_id, outfit_head, outfit_body, outfit_legs, outfit_feet) \
         VALUES ($1, 'Rizael', 0, 1, 1028, 1028, 7, 1028, 1028, 7, \
                 2, 150, 150, 0, 0, 400, 400, 133, 1, 2, 3, 4) \
         RETURNING id",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let insert = "INSERT INTO game_tokens (token_hash, character_id, valid_until) \
                  VALUES ($1, $2, NOW() + INTERVAL '60 seconds')";

    sqlx::query(insert)
        .bind("hash-one")
        .bind(character_id)
        .execute(&pool)
        .await
        .expect("the first token must insert");

    let second = sqlx::query(insert)
        .bind("hash-two")
        .bind(character_id)
        .execute(&pool)
        .await;

    // Asserting on the specific constraint, not merely on `is_err`. A bare `is_err`
    // would stay green if this ever started failing for an unrelated reason — a
    // reshaped column, a different constraint firing first — and the invariant this
    // test exists to guard could be gone without anyone noticing.
    let err = second.expect_err("a second token for the same character must be refused");
    let db_err = err
        .as_database_error()
        .expect("the refusal must come from the database, not from sqlx");

    assert_eq!(
        db_err.code().as_deref(),
        Some("23505"),
        "must fail as unique_violation, got {db_err:?}"
    );
    assert_eq!(
        db_err.constraint(),
        Some("game_tokens_character_id_key"),
        "the UNIQUE on character_id must be what refuses it"
    );
}

/// Tokens are stored hashed, so a plaintext `token` column must not come back — via a
/// reverted migration, a merge, or a hand-applied fix. If it does, some code path is
/// storing a credential again.
#[sqlx::test(migrations = "./migrations")]
async fn no_token_table_has_a_plaintext_token_column(pool: PgPool) {
    for table in ["sessions", "game_tokens"] {
        assert_eq!(
            column_type(&pool, table, "token").await,
            None,
            "{table}.token must not exist; only its SHA-256 digest is stored"
        );
        assert_column(&pool, table, "token_hash", "text").await;
    }
}
