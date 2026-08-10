use sqlx::PgPool;
use time::OffsetDateTime;

use crate::{
    domain::{sex::Sex, vocation::Vocation},
    error::AppError,
};

/// `player_skills.skill_type` for Level, matching the game server's `SkillType::Level`.
const SKILL_TYPE_LEVEL: i16 = 0;

#[derive(Debug, Clone)]
pub struct CharacterDetail {
    pub name: String,
    pub vocation: Vocation,
    pub sex: Sex,
    pub level: i16,
    pub online: bool,
    pub created_at: OffsetDateTime,
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub struct OnlineCharacter {
    pub name: String,
    pub vocation: Vocation,
    pub level: i16,
}

#[derive(Debug, Clone)]
pub struct HighscoreEntry {
    pub rank: i64,
    pub name: String,
    pub vocation: Vocation,
    pub level: i16,
}

/// Looks a character up by name, case-insensitively.
///
/// Deleted characters resolve too — the page shows a "this character has been
/// deleted" banner rather than pretending it never existed, which keeps old links
/// working and matches how tibia.com behaves.
pub async fn find_character_by_name(
    pool: &PgPool,
    name: &str,
) -> Result<Option<CharacterDetail>, AppError> {
    let row = sqlx::query_as::<_, (String, i16, i16, Option<i16>, bool, OffsetDateTime, bool)>(
        "SELECT p.name, p.vocation, p.sex, s.value, (o.character_id IS NOT NULL), \
                p.created_at, (p.deleted_at IS NOT NULL) \
         FROM players p \
         LEFT JOIN player_skills s ON s.player_id = p.id AND s.skill_type = $2 \
         LEFT JOIN online_players o ON o.character_id = p.id \
         WHERE lower(p.name) = lower($1)",
    )
    .bind(name)
    .bind(SKILL_TYPE_LEVEL)
    .fetch_optional(pool)
    .await?;

    row.map(|(name, vocation, sex, level, online, created_at, deleted)| {
        Ok(CharacterDetail {
            name,
            vocation: Vocation::from_i16(vocation)?,
            sex: Sex::from_i16(sex)?,
            level: level.unwrap_or(1),
            online,
            created_at,
            deleted,
        })
    })
    .transpose()
}

pub async fn who_is_online(pool: &PgPool) -> Result<Vec<OnlineCharacter>, AppError> {
    let rows = sqlx::query_as::<_, (String, i16, Option<i16>)>(
        "SELECT p.name, p.vocation, s.value \
         FROM online_players o \
         JOIN players p ON p.id = o.character_id \
         LEFT JOIN player_skills s ON s.player_id = p.id AND s.skill_type = $1 \
         WHERE p.deleted_at IS NULL \
         ORDER BY p.name",
    )
    .bind(SKILL_TYPE_LEVEL)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(name, vocation, level)| {
            Ok(OnlineCharacter {
                name,
                vocation: Vocation::from_i16(vocation)?,
                level: level.unwrap_or(1),
            })
        })
        .collect()
}

/// Top `limit` characters by level, highest first. Excludes deleted characters.
pub async fn highscores(pool: &PgPool, limit: i64) -> Result<Vec<HighscoreEntry>, AppError> {
    let rows = sqlx::query_as::<_, (String, i16, i16)>(
        "SELECT p.name, p.vocation, s.value \
         FROM players p \
         JOIN player_skills s ON s.player_id = p.id AND s.skill_type = $1 \
         WHERE p.deleted_at IS NULL \
         ORDER BY s.value DESC, p.name ASC \
         LIMIT $2",
    )
    .bind(SKILL_TYPE_LEVEL)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .enumerate()
        .map(|(index, (name, vocation, level))| {
            Ok(HighscoreEntry {
                rank: index as i64 + 1,
                name,
                vocation: Vocation::from_i16(vocation)?,
                level,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::SiteConfig,
        db::{accounts::create_account, characters},
    };

    async fn a_character(pool: &PgPool, email: &str, name: &str, level: i16) -> i32 {
        let template = SiteConfig::load("config.yaml").unwrap().new_character;
        let account = create_account(pool, email, "hunter2hunter2").await.unwrap();
        let id = characters::create(pool, account.id, name, Vocation::Knight, Sex::Male, &template)
            .await
            .unwrap();

        sqlx::query("UPDATE player_skills SET value = $2 WHERE player_id = $1 AND skill_type = 0")
            .bind(id)
            .bind(level)
            .execute(pool)
            .await
            .unwrap();

        id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn finds_a_character_case_insensitively(pool: PgPool) {
        a_character(&pool, "a@example.com", "Rizael", 8).await;

        let found = find_character_by_name(&pool, "rIzAeL").await.unwrap().unwrap();

        assert_eq!(found.name, "Rizael", "the stored capitalisation is returned");
        assert_eq!(found.level, 8);
        assert_eq!(found.vocation, Vocation::Knight);
        assert!(!found.deleted);
        assert!(!found.online);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unknown_name_is_none(pool: PgPool) {
        assert!(find_character_by_name(&pool, "Nobody").await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_deleted_character_still_resolves_and_is_flagged(pool: PgPool) {
        let id = a_character(&pool, "a@example.com", "Rizael", 5).await;
        sqlx::query("UPDATE players SET deleted_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        let found = find_character_by_name(&pool, "Rizael").await.unwrap().unwrap();

        assert!(found.deleted, "old links must still resolve, with a banner");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn who_is_online_lists_only_connected_characters(pool: PgPool) {
        let online = a_character(&pool, "a@example.com", "Rizael", 3).await;
        a_character(&pool, "b@example.com", "Elyra", 4).await;

        sqlx::query("INSERT INTO online_players (character_id) VALUES ($1)")
            .bind(online)
            .execute(&pool)
            .await
            .unwrap();

        let listed = who_is_online(&pool).await.unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Rizael");
        assert_eq!(listed[0].level, 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn who_is_online_excludes_deleted_characters(pool: PgPool) {
        let id = a_character(&pool, "a@example.com", "Rizael", 3).await;
        sqlx::query("INSERT INTO online_players (character_id) VALUES ($1)")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE players SET deleted_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        assert!(who_is_online(&pool).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn highscores_rank_by_level_descending(pool: PgPool) {
        a_character(&pool, "a@example.com", "Low", 2).await;
        a_character(&pool, "b@example.com", "High", 30).await;
        a_character(&pool, "c@example.com", "Mid", 10).await;

        let ranked = highscores(&pool, 100).await.unwrap();

        assert_eq!(ranked.len(), 3);
        assert_eq!((ranked[0].rank, ranked[0].name.as_str(), ranked[0].level), (1, "High", 30));
        assert_eq!((ranked[1].rank, ranked[1].name.as_str(), ranked[1].level), (2, "Mid", 10));
        assert_eq!((ranked[2].rank, ranked[2].name.as_str(), ranked[2].level), (3, "Low", 2));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn highscores_break_ties_by_name_so_the_order_is_stable(pool: PgPool) {
        a_character(&pool, "a@example.com", "Zeta", 5).await;
        a_character(&pool, "b@example.com", "Alpha", 5).await;

        let ranked = highscores(&pool, 100).await.unwrap();

        assert_eq!(ranked[0].name, "Alpha", "equal levels must order by name, not at random");
        assert_eq!(ranked[1].name, "Zeta");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn highscores_exclude_deleted_characters_and_honour_the_limit(pool: PgPool) {
        let deleted = a_character(&pool, "a@example.com", "Gone", 99).await;
        a_character(&pool, "b@example.com", "Kept", 1).await;
        sqlx::query("UPDATE players SET deleted_at = NOW() WHERE id = $1")
            .bind(deleted)
            .execute(&pool)
            .await
            .unwrap();

        let ranked = highscores(&pool, 100).await.unwrap();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].name, "Kept");

        a_character(&pool, "c@example.com", "Another", 50).await;
        assert_eq!(highscores(&pool, 1).await.unwrap().len(), 1, "the limit must be honoured");
    }
}
