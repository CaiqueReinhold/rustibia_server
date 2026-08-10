use sqlx::PgPool;
use thiserror::Error;

pub type AccountId = i32;

#[derive(Error, Debug)]
pub enum AuthRepositoryError {
    #[error("Token not found or expired")]
    InvalidToken,
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
}

pub struct AuthRepository {
    pool: PgPool,
}

impl AuthRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn validate_token(&self, token: &str) -> Result<AccountId, AuthRepositoryError> {
        use sqlx::Row;

        let row = sqlx::query(
            "SELECT account_id FROM auth_tokens WHERE token = $1 AND valid_until > NOW()",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AuthRepositoryError::InvalidToken)?;

        Ok(row.try_get::<i32, _>("account_id")?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inserts an account and returns its id. `accounts.id` is GENERATED ALWAYS,
    /// so the id cannot be chosen — it must be read back.
    async fn insert_account(pool: &PgPool) -> i32 {
        sqlx::query_scalar::<_, i32>(
            "INSERT INTO accounts (email, password_hash) VALUES ($1, $2) RETURNING id",
        )
        .bind("fixture@example.com")
        .bind("not-a-real-hash")
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../site/migrations")]
    async fn validate_token_returns_invalid_for_missing_token(pool: PgPool) {
        let repo = AuthRepository::new(pool);
        let result = repo.validate_token("nonexistent").await;
        assert!(matches!(result, Err(AuthRepositoryError::InvalidToken)));
    }

    #[sqlx::test(migrations = "../site/migrations")]
    async fn validate_token_returns_invalid_for_expired_token(pool: PgPool) {
        let account_id = insert_account(&pool).await;

        sqlx::query(
            "INSERT INTO auth_tokens (token, account_id, valid_until) \
             VALUES ($1, $2, NOW() - INTERVAL '1 hour')",
        )
        .bind("expired_token")
        .bind(account_id)
        .execute(&pool)
        .await
        .unwrap();

        let repo = AuthRepository::new(pool);
        let result = repo.validate_token("expired_token").await;
        assert!(matches!(result, Err(AuthRepositoryError::InvalidToken)));
    }

    #[sqlx::test(migrations = "../site/migrations")]
    async fn validate_token_returns_account_id_for_valid_token(pool: PgPool) {
        let account_id = insert_account(&pool).await;

        sqlx::query(
            "INSERT INTO auth_tokens (token, account_id, valid_until) \
             VALUES ($1, $2, NOW() + INTERVAL '1 hour')",
        )
        .bind("valid_token")
        .bind(account_id)
        .execute(&pool)
        .await
        .unwrap();

        let repo = AuthRepository::new(pool);
        let result = repo.validate_token("valid_token").await.unwrap();
        assert_eq!(result, account_id);
    }
}
