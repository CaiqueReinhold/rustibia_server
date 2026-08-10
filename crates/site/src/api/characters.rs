use axum::{Json, extract::Path, extract::State};
use serde::Serialize;
use time::{Duration, OffsetDateTime};

use crate::{
    auth::{extractor::CurrentAccount, token::generate_token},
    db::characters,
    error::AppError,
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct CharacterSummary {
    pub id: i32,
    pub name: String,
    pub level: i16,
    pub vocation: String,
}

/// The character list the game client shows after login.
///
/// `sex` and `online` are deliberately absent — the client's character-select screen
/// shows name, level and vocation, and has no use for either. The website's dashboard
/// renders online status by querying the database directly.
pub async fn get_characters(
    State(state): State<AppState>,
    account: CurrentAccount,
) -> Result<Json<Vec<CharacterSummary>>, AppError> {
    let characters = characters::list_for_account(&state.pool, account.account_id).await?;

    Ok(Json(
        characters
            .into_iter()
            .map(|c| CharacterSummary {
                id: c.id,
                name: c.name,
                level: c.level,
                vocation: c.vocation.name().to_string(),
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize)]
pub struct GameTokenResponse {
    pub auth_token: String,
    pub expires_at: String,
}

/// Issues a short-lived token the game client hands to the game server on connect.
///
/// The TTL is deliberately tiny: the client redeems it within seconds. It is NOT
/// single-use — the game server's `validate_token` only checks expiry and never
/// deletes — so the short window is the whole mitigation. Making it one-shot needs
/// a game-server change and is recorded as deferred in the spec.
pub async fn post_character_token(
    State(state): State<AppState>,
    account: CurrentAccount,
    Path(character_id): Path<i32>,
) -> Result<Json<GameTokenResponse>, AppError> {
    if !characters::belongs_to_account(&state.pool, character_id, account.account_id).await? {
        // Same answer whether the character is absent, deleted, or someone else's,
        // so this cannot be used to enumerate character ids.
        return Err(AppError::NotFound);
    }

    let token = generate_token();
    let valid_until =
        OffsetDateTime::now_utc() + Duration::seconds(state.config.auth_token_ttl_seconds);

    sqlx::query("INSERT INTO auth_tokens (token, account_id, valid_until) VALUES ($1, $2, $3)")
        .bind(&token)
        .bind(account.account_id)
        .bind(valid_until)
        .execute(&state.pool)
        .await?;

    Ok(Json(GameTokenResponse {
        auth_token: token,
        expires_at: valid_until
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::SiteConfig,
        db::{accounts::create_account, sessions},
        domain::{sex::Sex, vocation::Vocation},
    };
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        routing::{get, post},
    };
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn test_app(pool: PgPool) -> Router {
        let config = SiteConfig::load("config.yaml").unwrap();
        Router::new()
            .route("/api/characters", get(get_characters))
            .route("/api/characters/{id}/token", post(post_character_token))
            .with_state(AppState { pool, config })
    }

    async fn send(app: Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    fn get_list(token: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri("/api/characters");
        if let Some(t) = token {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        b.body(Body::empty()).unwrap()
    }

    fn post_token_req(character_id: i32, token: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri(format!("/api/characters/{character_id}/token"));
        if let Some(t) = token {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        b.body(Body::empty()).unwrap()
    }

    async fn account_with_session(pool: &PgPool, email: &str) -> (i32, String) {
        let account = create_account(pool, email, "hunter2hunter2").await.unwrap();
        let session = sessions::issue(pool, account.id, 7).await.unwrap();
        (account.id, session.token)
    }

    async fn a_character(pool: &PgPool, account_id: i32, name: &str) -> i32 {
        let template = SiteConfig::load("config.yaml").unwrap().new_character;
        characters::create(pool, account_id, name, Vocation::Paladin, Sex::Male, &template)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn returns_the_accounts_characters(pool: PgPool) {
        let (account_id, token) = account_with_session(&pool, "player@example.com").await;
        a_character(&pool, account_id, "Rizael").await;

        let (status, body) = send(test_app(pool), get_list(Some(&token))).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body[0]["name"], "Rizael");
        assert_eq!(body[0]["vocation"], "Paladin");
        assert_eq!(body[0]["level"], 1);
        assert!(body[0]["sex"].is_null(), "sex must not be exposed by this endpoint");
        assert!(body[0]["online"].is_null(), "online must not be exposed by this endpoint");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_missing_token_is_401(pool: PgPool) {
        let (status, _) = send(test_app(pool), get_list(None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unknown_token_is_401(pool: PgPool) {
        let (status, _) = send(test_app(pool), get_list(Some("not-a-real-token"))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn one_account_never_sees_anothers_characters(pool: PgPool) {
        let (owner_id, _) = account_with_session(&pool, "owner@example.com").await;
        let (_, stranger_token) = account_with_session(&pool, "stranger@example.com").await;
        a_character(&pool, owner_id, "Rizael").await;

        let (status, body) = send(test_app(pool), get_list(Some(&stranger_token))).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 0, "must not leak another account's characters");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn deleted_characters_are_excluded(pool: PgPool) {
        let (account_id, token) = account_with_session(&pool, "player@example.com").await;
        let id = a_character(&pool, account_id, "Rizael").await;
        characters::soft_delete(&pool, id, account_id).await.unwrap();

        let (_, body) = send(test_app(pool), get_list(Some(&token))).await;

        assert_eq!(body.as_array().unwrap().len(), 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn issues_a_token_the_game_server_would_accept(pool: PgPool) {
        let (account_id, token) = account_with_session(&pool, "player@example.com").await;
        let character_id = a_character(&pool, account_id, "Rizael").await;

        let (status, body) = send(test_app(pool.clone()), post_token_req(character_id, Some(&token))).await;

        assert_eq!(status, StatusCode::OK);
        let auth_token = body["auth_token"].as_str().unwrap();
        assert_eq!(auth_token.len(), 64);

        // The game server's own query, verbatim from
        // game_server/src/persistence/auth.rs:29-32.
        let resolved: Option<(i32,)> = sqlx::query_as(
            "SELECT account_id FROM auth_tokens WHERE token = $1 AND valid_until > NOW()",
        )
        .bind(auth_token)
        .fetch_optional(&pool)
        .await
        .unwrap();

        assert_eq!(
            resolved.map(|r| r.0),
            Some(account_id),
            "the issued token must resolve through the game server's own lookup"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn another_accounts_character_is_404_and_mints_nothing(pool: PgPool) {
        let (owner_id, _) = account_with_session(&pool, "owner@example.com").await;
        let (_, stranger_token) = account_with_session(&pool, "stranger@example.com").await;
        let character_id = a_character(&pool, owner_id, "Rizael").await;

        let (status, _) = send(test_app(pool.clone()), post_token_req(character_id, Some(&stranger_token))).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "no token may be minted for someone else's character");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_nonexistent_character_is_404_not_500(pool: PgPool) {
        let (_, token) = account_with_session(&pool, "player@example.com").await;

        let (status, _) = send(test_app(pool), post_token_req(999_999, Some(&token))).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_deleted_character_cannot_get_a_token(pool: PgPool) {
        let (account_id, token) = account_with_session(&pool, "player@example.com").await;
        let character_id = a_character(&pool, account_id, "Rizael").await;
        characters::soft_delete(&pool, character_id, account_id).await.unwrap();

        let (status, _) = send(test_app(pool), post_token_req(character_id, Some(&token))).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn no_session_is_401(pool: PgPool) {
        let (status, _) = send(test_app(pool), post_token_req(1, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
