use sqlx::PgPool;
use time::OffsetDateTime;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct NewsPost {
    pub id: i32,
    pub title: String,
    pub body: String,
    pub posted_at: OffsetDateTime,
}

/// Most recent posts first.
///
/// The author is deliberately not selected. Accounts have no display name, so the
/// only thing available is the admin's email address, and publishing that on the
/// homepage would hand out a login identifier to anyone reading the news.
pub async fn list_recent(pool: &PgPool, limit: i64) -> Result<Vec<NewsPost>, AppError> {
    let rows = sqlx::query_as::<_, (i32, String, String, OffsetDateTime)>(
        "SELECT id, title, body, posted_at FROM news_posts ORDER BY posted_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, title, body, posted_at)| NewsPost { id, title, body, posted_at })
        .collect())
}

pub async fn create(
    pool: &PgPool,
    title: &str,
    body: &str,
    author_id: i32,
) -> Result<i32, AppError> {
    let title = title.trim();
    let body = body.trim();

    if title.is_empty() {
        return Err(AppError::Validation("A news post needs a title.".to_string()));
    }
    if body.is_empty() {
        return Err(AppError::Validation("A news post needs a body.".to_string()));
    }

    let id = sqlx::query_scalar::<_, i32>(
        "INSERT INTO news_posts (title, body, author_id) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(title)
    .bind(body)
    .bind(author_id)
    .fetch_one(pool)
    .await?;

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::accounts::create_account;

    async fn an_admin(pool: &PgPool) -> i32 {
        let account = create_account(pool, "admin@example.com", "hunter2hunter2").await.unwrap();
        sqlx::query("UPDATE accounts SET is_admin = TRUE WHERE id = $1")
            .bind(account.id)
            .execute(pool)
            .await
            .unwrap();
        account.id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn creates_and_lists_a_post(pool: PgPool) {
        let author = an_admin(&pool).await;

        create(&pool, "Server is open", "Come and play.", author).await.unwrap();

        let posts = list_recent(&pool, 10).await.unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].title, "Server is open");
        assert_eq!(posts[0].body, "Come and play.");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn lists_newest_first_and_honours_the_limit(pool: PgPool) {
        let author = an_admin(&pool).await;

        for n in 1..=3 {
            create(&pool, &format!("Post {n}"), "body", author).await.unwrap();
            // posted_at defaults to NOW(); nudge each row forward so the ordering is
            // deterministic rather than dependent on clock resolution.
            sqlx::query("UPDATE news_posts SET posted_at = NOW() + ($1 || ' seconds')::interval WHERE title = $2")
                .bind(n.to_string())
                .bind(format!("Post {n}"))
                .execute(&pool)
                .await
                .unwrap();
        }

        let posts = list_recent(&pool, 2).await.unwrap();

        assert_eq!(posts.len(), 2, "the limit must be honoured");
        assert_eq!(posts[0].title, "Post 3", "newest first");
        assert_eq!(posts[1].title, "Post 2");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rejects_an_empty_title_or_body(pool: PgPool) {
        let author = an_admin(&pool).await;

        assert!(matches!(
            create(&pool, "   ", "body", author).await,
            Err(AppError::Validation(_))
        ));
        assert!(matches!(
            create(&pool, "title", "\n\t ", author).await,
            Err(AppError::Validation(_))
        ));

        assert_eq!(list_recent(&pool, 10).await.unwrap().len(), 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_news_table_lists_nothing(pool: PgPool) {
        assert!(list_recent(&pool, 10).await.unwrap().is_empty());
    }
}
