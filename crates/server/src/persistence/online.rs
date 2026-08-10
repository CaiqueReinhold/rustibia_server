use sqlx::PgPool;

/// Writes the `online_players` table that the website reads for its player count and
/// Who Is Online list. Owned by the game server because only it knows who is actually
/// connected; the schema itself is owned by `game_site`.
pub struct OnlineRepository {
    pool: PgPool,
}

impl OnlineRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn mark_online(&self, character_id: u32) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO online_players (character_id) VALUES ($1) \
             ON CONFLICT (character_id) DO NOTHING",
        )
        .bind(character_id as i32)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_offline(&self, character_id: u32) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM online_players WHERE character_id = $1")
            .bind(character_id as i32)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Nobody is online when the process starts, so any surviving rows are debris
    /// from an unclean shutdown. Clearing at boot bounds how long drift can last.
    pub async fn clear_all(&self) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM online_players")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn a_character(pool: &PgPool) -> i32 {
        let account_id: i32 = sqlx::query_scalar(
            "INSERT INTO accounts (email, password_hash) VALUES ($1, $2) RETURNING id",
        )
        .bind(format!("fixture-{}@example.com", uuid::Uuid::now_v7()))
        .bind("not-a-real-hash")
        .fetch_one(pool)
        .await
        .unwrap();

        sqlx::query_scalar::<_, i32>(
            "INSERT INTO players \
             (account_id, name, vocation, sex, pos_x, pos_y, pos_z, origin_x, origin_y, origin_z, \
              facing, life_cur, life_max, mana_cur, mana_max, cap_cur, cap_max, \
              outfit_id, outfit_head, outfit_body, outfit_legs, outfit_feet) \
             VALUES ($1, $2, 0, 1, 1028, 1028, 7, 1028, 1028, 7, \
                     2, 150, 150, 0, 0, 400, 400, 128, 78, 69, 58, 76) \
             RETURNING id",
        )
        .bind(account_id)
        .bind(format!(
            "Fixture{}",
            uuid::Uuid::now_v7().as_u128() % 100000
        ))
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn online_count(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM online_players")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "../site/migrations")]
    async fn marks_a_character_online_then_offline(pool: PgPool) {
        let id = a_character(&pool).await;
        let repo = OnlineRepository::new(pool.clone());

        repo.mark_online(id as u32).await.unwrap();
        assert_eq!(online_count(&pool).await, 1);

        repo.mark_offline(id as u32).await.unwrap();
        assert_eq!(online_count(&pool).await, 0);
    }

    #[sqlx::test(migrations = "../site/migrations")]
    async fn marking_online_twice_is_harmless(pool: PgPool) {
        let id = a_character(&pool).await;
        let repo = OnlineRepository::new(pool.clone());

        repo.mark_online(id as u32).await.unwrap();
        repo.mark_online(id as u32).await.unwrap();

        assert_eq!(
            online_count(&pool).await,
            1,
            "a duplicate register must not error or double-count"
        );
    }

    #[sqlx::test(migrations = "../site/migrations")]
    async fn clear_all_empties_the_table(pool: PgPool) {
        let repo = OnlineRepository::new(pool.clone());
        for _ in 0..3 {
            let id = a_character(&pool).await;
            repo.mark_online(id as u32).await.unwrap();
        }

        repo.clear_all().await.unwrap();

        assert_eq!(online_count(&pool).await, 0);
    }
}
