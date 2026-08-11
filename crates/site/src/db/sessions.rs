use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use crate::{
    auth::token::{generate_token, hash_token},
    error::AppError,
};

#[derive(Debug, Clone)]
pub struct Session {
    pub token: String,
    // pub account_id: i32,
    pub valid_until: OffsetDateTime,
}

pub async fn issue(pool: &PgPool, account_id: i32, ttl_days: i64) -> Result<Session, AppError> {
    let token = generate_token();
    let valid_until = OffsetDateTime::now_utc() + Duration::days(ttl_days);

    sqlx::query("INSERT INTO sessions (token_hash, account_id, valid_until) VALUES ($1, $2, $3)")
        .bind(hash_token(&token))
        .bind(account_id)
        .bind(valid_until)
        .execute(pool)
        .await?;

    Ok(Session {
        token,
        // account_id,
        valid_until,
    })
}

/// Returns the account id for a live session, or `None` if the token is unknown or expired.
pub async fn account_for_token(pool: &PgPool, token: &str) -> Result<Option<i32>, AppError> {
    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT account_id FROM sessions WHERE token_hash = $1 AND valid_until > NOW()",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| r.0))
}

pub async fn revoke(pool: &PgPool, token: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(hash_token(token))
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::accounts::create_account;

    async fn an_account(pool: &PgPool) -> i32 {
        create_account(pool, "player@example.com", "hunter2hunter2")
            .await
            .unwrap()
            .id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn issued_session_resolves_to_its_account(pool: PgPool) {
        let account_id = an_account(&pool).await;
        let session = issue(&pool, account_id, 7).await.unwrap();

        let resolved = account_for_token(&pool, &session.token).await.unwrap();

        assert_eq!(resolved, Some(account_id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_token_resolves_to_none(pool: PgPool) {
        assert_eq!(account_for_token(&pool, "nope").await.unwrap(), None);
    }

    /// A read of this table must not yield anything that can be presented as a session.
    #[sqlx::test(migrations = "./migrations")]
    async fn the_session_token_is_never_stored_in_the_clear(pool: PgPool) {
        let account_id = an_account(&pool).await;
        let session = issue(&pool, account_id, 7).await.unwrap();

        let stored: Vec<String> = sqlx::query_scalar("SELECT token_hash FROM sessions")
            .fetch_all(&pool)
            .await
            .unwrap();

        assert_eq!(stored.len(), 1);
        assert_ne!(stored[0], session.token);
        assert_eq!(stored[0], hash_token(&session.token));
    }

    /// The digest is not a credential: presenting it where the token belongs must fail.
    /// Otherwise hashing would accomplish nothing, since a reader of the table would hold
    /// a usable value after all.
    #[sqlx::test(migrations = "./migrations")]
    async fn the_stored_hash_cannot_itself_be_used_as_a_session(pool: PgPool) {
        let account_id = an_account(&pool).await;
        let session = issue(&pool, account_id, 7).await.unwrap();

        let resolved = account_for_token(&pool, &hash_token(&session.token))
            .await
            .unwrap();

        assert_eq!(
            resolved, None,
            "the value in the table must not authenticate anyone"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn expired_session_resolves_to_none(pool: PgPool) {
        let account_id = an_account(&pool).await;
        let session = issue(&pool, account_id, -1).await.unwrap();

        assert_eq!(
            account_for_token(&pool, &session.token).await.unwrap(),
            None
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn revoked_session_resolves_to_none(pool: PgPool) {
        let account_id = an_account(&pool).await;
        let session = issue(&pool, account_id, 7).await.unwrap();

        revoke(&pool, &session.token).await.unwrap();

        assert_eq!(
            account_for_token(&pool, &session.token).await.unwrap(),
            None
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_the_account_deletes_its_sessions(pool: PgPool) {
        let account_id = an_account(&pool).await;
        let session = issue(&pool, account_id, 7).await.unwrap();

        sqlx::query("DELETE FROM accounts WHERE id = $1")
            .bind(account_id)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            account_for_token(&pool, &session.token).await.unwrap(),
            None,
            "ON DELETE CASCADE must remove the session row"
        );
    }
}
