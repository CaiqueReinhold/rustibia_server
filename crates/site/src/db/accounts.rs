use sqlx::PgPool;

use crate::{auth::password::hash_password, error::AppError};

#[derive(Debug, Clone)]
pub struct Account {
    pub id: i32,
    pub email: String,
    pub password_hash: String,
    pub is_admin: bool,
}

/// Inserts an account. Returns `AppError::Validation` if the email is already taken,
/// distinguishing that from a genuine database failure via the unique-violation code.
pub async fn create_account(
    pool: &PgPool,
    email: &str,
    plain_password: &str,
) -> Result<Account, AppError> {
    let hash = hash_password(plain_password)?;

    let row = sqlx::query_as::<_, (i32, String, String, bool)>(
        "INSERT INTO accounts (email, password_hash) VALUES ($1, $2) \
         RETURNING id, email, password_hash, is_admin",
    )
    .bind(email)
    .bind(&hash)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AppError::Validation("That email address is already registered.".to_string())
        }
        _ => AppError::Database(e),
    })?;

    Ok(Account { id: row.0, email: row.1, password_hash: row.2, is_admin: row.3 })
}

pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<Account>, AppError> {
    let row = sqlx::query_as::<_, (i32, String, String, bool)>(
        "SELECT id, email, password_hash, is_admin FROM accounts WHERE lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| Account { id: r.0, email: r.1, password_hash: r.2, is_admin: r.3 }))
}

pub async fn find_by_id(pool: &PgPool, account_id: i32) -> Result<Option<Account>, AppError> {
    let row = sqlx::query_as::<_, (i32, String, String, bool)>(
        "SELECT id, email, password_hash, is_admin FROM accounts WHERE id = $1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| Account { id: r.0, email: r.1, password_hash: r.2, is_admin: r.3 }))
}

/// Sets a new password and revokes every session except the one making the change.
///
/// Revocation is the point: if the password is being changed because it leaked, any
/// session an attacker already holds must stop working. Keeping the current session
/// alive means the user is not logged out of the tab they are using.
pub async fn update_password(
    pool: &PgPool,
    account_id: i32,
    new_plain_password: &str,
    keep_session_token: &str,
) -> Result<(), AppError> {
    let hash = hash_password(new_plain_password)?;

    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE accounts SET password_hash = $2 WHERE id = $1")
        .bind(account_id)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM sessions WHERE account_id = $1 AND token <> $2")
        .bind(account_id)
        .bind(keep_session_token)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::password::verify_password;

    #[sqlx::test(migrations = "./migrations")]
    async fn creates_an_account_and_hashes_the_password(pool: PgPool) {
        let account = create_account(&pool, "Player@Example.com", "hunter2hunter2")
            .await
            .unwrap();

        assert!(account.id > 0, "id must come from the identity sequence");
        assert!(!account.is_admin, "new accounts are never admins");
        assert_ne!(account.password_hash, "hunter2hunter2", "must not store plaintext");
        assert!(verify_password("hunter2hunter2", &account.password_hash));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rejects_a_duplicate_email_case_insensitively(pool: PgPool) {
        create_account(&pool, "player@example.com", "hunter2hunter2").await.unwrap();

        let err = create_account(&pool, "PLAYER@EXAMPLE.COM", "different").await.unwrap_err();

        assert!(
            matches!(err, AppError::Validation(_)),
            "duplicate email must be a validation error, got {err:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn finds_an_account_regardless_of_email_case(pool: PgPool) {
        let created = create_account(&pool, "player@example.com", "hunter2hunter2")
            .await
            .unwrap();

        let found = find_by_email(&pool, "PLAYER@example.COM").await.unwrap().unwrap();

        assert_eq!(found.id, created.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn returns_none_for_an_unknown_email(pool: PgPool) {
        assert!(find_by_email(&pool, "nobody@example.com").await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn changing_the_password_revokes_other_sessions_but_keeps_this_one(pool: PgPool) {
        use crate::db::sessions;

        let account = create_account(&pool, "player@example.com", "hunter2hunter2").await.unwrap();
        let keep = sessions::issue(&pool, account.id, 7).await.unwrap();
        let stolen = sessions::issue(&pool, account.id, 7).await.unwrap();

        update_password(&pool, account.id, "a-brand-new-password", &keep.token).await.unwrap();

        assert_eq!(
            sessions::account_for_token(&pool, &stolen.token).await.unwrap(),
            None,
            "a session an attacker already holds must stop working after a password change"
        );
        assert_eq!(
            sessions::account_for_token(&pool, &keep.token).await.unwrap(),
            Some(account.id),
            "the session making the change must survive"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_new_password_replaces_the_old_one(pool: PgPool) {
        use crate::auth::password::verify_password;

        let account = create_account(&pool, "player@example.com", "hunter2hunter2").await.unwrap();

        update_password(&pool, account.id, "a-brand-new-password", "irrelevant").await.unwrap();

        let reloaded = find_by_id(&pool, account.id).await.unwrap().unwrap();
        assert!(verify_password("a-brand-new-password", &reloaded.password_hash));
        assert!(
            !verify_password("hunter2hunter2", &reloaded.password_hash),
            "the old password must stop working"
        );
    }
}
