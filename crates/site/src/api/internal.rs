//! The API the game server calls, and nobody else.

use axum::{Json, extract::State};
use rustibia_contract::{CharacterRecord, RedeemRequest};

use crate::{db::login, error::AppError, state::AppState};

/// Spends an auth token and returns the character the game server should load.
pub async fn post_redeem(
    State(state): State<AppState>,
    Json(request): Json<RedeemRequest>,
) -> Result<Json<CharacterRecord>, AppError> {
    match login::redeem(&state.pool, &request.auth_token, request.character_id).await? {
        Some(record) => Ok(Json(record)),
        None => Err(AppError::NotFound),
    }
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/sessions/redeem", axum::routing::post(post_redeem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::SiteConfig,
        db::{accounts::create_account, characters},
        domain::{sex::Sex, vocation::Vocation},
    };
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
    };
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use time::{Duration, OffsetDateTime};
    use tower::ServiceExt;

    /// The internal router with TLS bypassed. Every test here is about the handler's
    /// answers; that the route is unreachable without a client certificate is proved in
    /// `internal_tls`, where the listener actually exists.
    fn test_app(pool: PgPool) -> Router {
        let config = SiteConfig::load("config.yaml").unwrap();
        Router::new()
            .nest("/internal", router())
            .with_state(AppState { pool, config })
    }

    fn redeem_req(token: &str, character_id: i32) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/internal/sessions/redeem")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&RedeemRequest {
                    auth_token: token.to_string(),
                    character_id,
                })
                .unwrap(),
            ))
            .unwrap()
    }

    async fn send(app: Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    async fn an_account(pool: &PgPool, email: &str) -> i32 {
        create_account(pool, email, "hunter2hunter2")
            .await
            .unwrap()
            .id
    }

    async fn a_character(pool: &PgPool, account_id: i32, name: &str) -> i32 {
        let template = SiteConfig::load("config.yaml").unwrap().new_character;
        characters::create(
            pool,
            account_id,
            name,
            Vocation::Paladin,
            Sex::Male,
            &template,
        )
        .await
        .unwrap()
    }

    async fn a_token(pool: &PgPool, account_id: i32) -> String {
        let token = format!("token-{}", uuid::Uuid::now_v7());
        sqlx::query(
            "INSERT INTO auth_tokens (token_hash, account_id, valid_until) VALUES ($1, $2, $3)",
        )
        .bind(crate::auth::token::hash_token(&token))
        .bind(account_id)
        .bind(OffsetDateTime::now_utc() + Duration::seconds(60))
        .execute(pool)
        .await
        .unwrap();
        token
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_valid_redemption_returns_the_record(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let character_id = a_character(&pool, account_id, "Rizael").await;
        let token = a_token(&pool, account_id).await;

        let (status, body) = send(test_app(pool), redeem_req(&token, character_id)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], character_id);
        assert_eq!(body["name"], "Rizael");
        assert!(body["position"]["x"].is_i64());
        assert!(!body["skills"].as_array().unwrap().is_empty());
    }

    /// The response has to deserialize into the contract type the game server parses,
    /// not merely into "some JSON". A field the site forgot to send would pass an
    /// assertion on `body["name"]` and fail on the game server at login time.
    #[sqlx::test(migrations = "./migrations")]
    async fn the_body_deserializes_as_the_contract_type(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let character_id = a_character(&pool, account_id, "Rizael").await;
        let token = a_token(&pool, account_id).await;

        let (_, body) = send(test_app(pool), redeem_req(&token, character_id)).await;

        let record: CharacterRecord = serde_json::from_value(body)
            .expect("the response must satisfy rustibia_contract::CharacterRecord");
        assert_eq!(record.id, character_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unknown_token_is_404(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let character_id = a_character(&pool, account_id, "Rizael").await;

        let (status, _) = send(test_app(pool), redeem_req("never-issued", character_id)).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_second_redemption_of_the_same_token_is_404(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let character_id = a_character(&pool, account_id, "Rizael").await;
        let token = a_token(&pool, account_id).await;

        let (first, _) = send(test_app(pool.clone()), redeem_req(&token, character_id)).await;
        let (second, _) = send(test_app(pool), redeem_req(&token, character_id)).await;

        assert_eq!(first, StatusCode::OK);
        assert_eq!(second, StatusCode::NOT_FOUND);
    }

    /// The id-oracle property: a bad token and a character that isn't yours must be
    /// byte-identical answers, or one valid token is enough to map out which character
    /// ids exist.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_bad_token_and_someone_elses_character_are_indistinguishable(pool: PgPool) {
        let owner_id = an_account(&pool, "owner@example.com").await;
        let stranger_id = an_account(&pool, "stranger@example.com").await;
        let character_id = a_character(&pool, owner_id, "Rizael").await;
        let stranger_token = a_token(&pool, stranger_id).await;

        let (bad_token_status, bad_token_body) = send(
            test_app(pool.clone()),
            redeem_req("never-issued", character_id),
        )
        .await;
        let (wrong_owner_status, wrong_owner_body) =
            send(test_app(pool), redeem_req(&stranger_token, character_id)).await;

        assert_eq!(bad_token_status, StatusCode::NOT_FOUND);
        assert_eq!(wrong_owner_status, StatusCode::NOT_FOUND);
        assert_eq!(
            bad_token_body, wrong_owner_body,
            "the two refusals must be indistinguishable"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_malformed_body_is_rejected(pool: PgPool) {
        let request = Request::builder()
            .method("POST")
            .uri("/internal/sessions/redeem")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"auth_token":"abc"}"#))
            .unwrap();

        let (status, _) = send(test_app(pool), request).await;

        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "a request missing character_id must not be treated as character 0"
        );
    }
}
