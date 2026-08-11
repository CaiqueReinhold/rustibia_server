use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::{
    auth::password::{spend_dummy_verification, verify_password},
    db::{accounts, sessions},
    error::AppError,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub session_token: String,
    pub expires_at: String,
}

pub async fn post_auth(
    State(state): State<AppState>,
    Json(req): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let account = accounts::find_by_email(&state.pool, &req.email).await?;

    let Some(account) = account else {
        // Spend equivalent work so timing does not reveal that the email is unknown.
        spend_dummy_verification(&req.password);
        return Err(AppError::InvalidCredentials);
    };

    if !verify_password(&req.password, &account.password_hash) {
        return Err(AppError::InvalidCredentials);
    }

    let session = sessions::issue(&state.pool, account.id, state.config.session_ttl_days).await?;

    Ok(Json(AuthResponse {
        session_token: session.token,
        expires_at: session
            .valid_until
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SiteConfig;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        routing::post,
    };
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn test_app(pool: PgPool) -> Router {
        let config = SiteConfig::load("config.yaml").unwrap();
        Router::new()
            .route("/api/auth", post(post_auth))
            .with_state(AppState { pool, config })
    }

    async fn post_json(
        app: Router,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn valid_credentials_return_a_session_token(pool: PgPool) {
        accounts::create_account(&pool, "player@example.com", "hunter2hunter2")
            .await
            .unwrap();

        let (status, body) = post_json(
            test_app(pool.clone()),
            "/api/auth",
            serde_json::json!({"email": "player@example.com", "password": "hunter2hunter2"}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let token = body["session_token"]
            .as_str()
            .expect("session_token present");
        assert_eq!(token.len(), 64);
        assert!(body["expires_at"].as_str().unwrap().contains('T'));

        let resolved = sessions::account_for_token(&pool, token).await.unwrap();
        assert!(
            resolved.is_some(),
            "returned token must resolve to an account"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn email_is_matched_case_insensitively(pool: PgPool) {
        accounts::create_account(&pool, "player@example.com", "hunter2hunter2")
            .await
            .unwrap();

        let (status, _) = post_json(
            test_app(pool),
            "/api/auth",
            serde_json::json!({"email": "PLAYER@EXAMPLE.COM", "password": "hunter2hunter2"}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn wrong_password_is_401(pool: PgPool) {
        accounts::create_account(&pool, "player@example.com", "hunter2hunter2")
            .await
            .unwrap();

        let (status, body) = post_json(
            test_app(pool),
            "/api/auth",
            serde_json::json!({"email": "player@example.com", "password": "wrong"}),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body["session_token"].is_null());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_email_is_401_with_the_identical_body(pool: PgPool) {
        accounts::create_account(&pool, "player@example.com", "hunter2hunter2")
            .await
            .unwrap();

        let (wrong_pw_status, wrong_pw_body) = post_json(
            test_app(pool.clone()),
            "/api/auth",
            serde_json::json!({"email": "player@example.com", "password": "wrong"}),
        )
        .await;

        let (unknown_status, unknown_body) = post_json(
            test_app(pool),
            "/api/auth",
            serde_json::json!({"email": "nobody@example.com", "password": "wrong"}),
        )
        .await;

        assert_eq!(wrong_pw_status, unknown_status);
        assert_eq!(
            wrong_pw_body, unknown_body,
            "the two failures must be indistinguishable to a client"
        );
    }
}
