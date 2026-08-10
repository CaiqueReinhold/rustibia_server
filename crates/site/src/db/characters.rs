use sqlx::PgPool;

use crate::{
    config::NewCharacterConfig,
    domain::{sex::Sex, vocation::Vocation},
    error::AppError,
};

#[derive(Debug, Clone)]
pub struct Character {
    pub id: i32,
    pub name: String,
    pub vocation: Vocation,
    pub sex: Sex,
    pub level: i16,
    pub online: bool,
}

/// The level skill's discriminant in `player_skills.skill_type`, matching the game
/// server's `SkillType::Level` (`game_server/src/persistence/player.rs:290-303`).
const SKILL_TYPE_LEVEL: i16 = 0;

/// Creates a character and its starting skills in one transaction.
///
/// The transaction matters: a `players` row without its `player_skills` rows would
/// load with an empty skill map and break the game server on first login.
pub async fn create(
    pool: &PgPool,
    account_id: i32,
    name: &str,
    vocation: Vocation,
    sex: Sex,
    template: &NewCharacterConfig,
) -> Result<i32, AppError> {
    let outfit_id = match sex {
        Sex::Female => template.outfit_id_female,
        Sex::Male => template.outfit_id_male,
    };

    let mut tx = pool.begin().await?;

    let character_id: i32 = sqlx::query_scalar(
        "INSERT INTO players \
         (account_id, name, vocation, sex, pos_x, pos_y, pos_z, origin_x, origin_y, origin_z, \
          facing, life_cur, life_max, mana_cur, mana_max, cap_cur, cap_max, \
          outfit_id, outfit_head, outfit_body, outfit_legs, outfit_feet) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$5,$6,$7,$8,$9,$9,$10,$10,$11,$11,$12,$13,$14,$15,$16) \
         RETURNING id",
    )
    .bind(account_id)
    .bind(name)
    .bind(vocation.as_i16())
    .bind(sex.as_i16())
    .bind(template.pos_x)
    .bind(template.pos_y)
    .bind(template.pos_z)
    .bind(template.facing)
    .bind(template.life)
    .bind(template.mana)
    .bind(template.capacity)
    .bind(outfit_id)
    .bind(template.outfit_head)
    .bind(template.outfit_body)
    .bind(template.outfit_legs)
    .bind(template.outfit_feet)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AppError::Validation("That character name is already taken.".to_string())
        }
        _ => AppError::Database(e),
    })?;

    for skill in &template.starting_skills {
        sqlx::query(
            "INSERT INTO player_skills (player_id, skill_type, value, current_ticks, max_ticks) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(character_id)
        .bind(skill.skill_type)
        .bind(skill.value)
        .bind(skill.current_ticks)
        .bind(skill.max_ticks)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(character_id)
}

/// Live characters for an account, with level and online status.
pub async fn list_for_account(pool: &PgPool, account_id: i32) -> Result<Vec<Character>, AppError> {
    let rows = sqlx::query_as::<_, (i32, String, i16, i16, Option<i16>, bool)>(
        "SELECT p.id, p.name, p.vocation, p.sex, s.value, (o.character_id IS NOT NULL) \
         FROM players p \
         LEFT JOIN player_skills s ON s.player_id = p.id AND s.skill_type = $2 \
         LEFT JOIN online_players o ON o.character_id = p.id \
         WHERE p.account_id = $1 AND p.deleted_at IS NULL \
         ORDER BY p.created_at",
    )
    .bind(account_id)
    .bind(SKILL_TYPE_LEVEL)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(id, name, vocation, sex, level, online)| {
            Ok(Character {
                id,
                name,
                vocation: Vocation::from_i16(vocation)?,
                sex: Sex::from_i16(sex)?,
                level: level.unwrap_or(1),
                online,
            })
        })
        .collect()
}

/// `true` if the character exists, is not deleted, and belongs to this account.
pub async fn belongs_to_account(
    pool: &PgPool,
    character_id: i32,
    account_id: i32,
) -> Result<bool, AppError> {
    let found: Option<(i32,)> = sqlx::query_as(
        "SELECT id FROM players WHERE id = $1 AND account_id = $2 AND deleted_at IS NULL",
    )
    .bind(character_id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?;

    Ok(found.is_some())
}

pub async fn is_online(pool: &PgPool, character_id: i32) -> Result<bool, AppError> {
    let found: Option<(i32,)> =
        sqlx::query_as("SELECT character_id FROM online_players WHERE character_id = $1")
            .bind(character_id)
            .fetch_optional(pool)
            .await?;

    Ok(found.is_some())
}

/// Soft-deletes a character. Returns `NotFound` if it is absent, already deleted,
/// owned by another account, **or currently online**.
///
/// The online condition is part of the UPDATE rather than a separate pre-check: a
/// character logging in between a check and the write would otherwise be deleted
/// while playing, and the game server's next save would write to a deleted row.
pub async fn soft_delete(
    pool: &PgPool,
    character_id: i32,
    account_id: i32,
) -> Result<(), AppError> {
    let result = sqlx::query(
        "UPDATE players SET deleted_at = NOW() \
         WHERE id = $1 AND account_id = $2 AND deleted_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM online_players o WHERE o.character_id = players.id)",
    )
    .bind(character_id)
    .bind(account_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::SiteConfig, db::accounts::create_account};

    fn template() -> NewCharacterConfig {
        SiteConfig::load("config.yaml").unwrap().new_character
    }

    async fn an_account(pool: &PgPool, email: &str) -> i32 {
        create_account(pool, email, "hunter2hunter2").await.unwrap().id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn creates_a_character_with_its_starting_skills(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;

        let id = create(&pool, account_id, "Rizael", Vocation::Knight, Sex::Male, &template())
            .await
            .unwrap();

        let skills: Vec<(i16, i16)> =
            sqlx::query_as("SELECT skill_type, value FROM player_skills WHERE player_id = $1 ORDER BY skill_type")
                .bind(id)
                .fetch_all(&pool)
                .await
                .unwrap();

        assert_eq!(skills.len(), 2, "both starting skills must be inserted");
        assert_eq!(skills[0], (0, 1), "Level starts at 1");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn origin_matches_position_on_a_fresh_character(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let id = create(&pool, account_id, "Rizael", Vocation::Druid, Sex::Female, &template())
            .await
            .unwrap();

        let row: (i32, i32, i32, i32) = sqlx::query_as(
            "SELECT pos_x, pos_y, origin_x, origin_y FROM players WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!((row.0, row.1), (row.2, row.3), "a new character starts at its origin");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sex_selects_the_outfit(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let t = template();

        let female = create(&pool, account_id, "Elyra", Vocation::Druid, Sex::Female, &t).await.unwrap();
        let male = create(&pool, account_id, "Bahrun", Vocation::Knight, Sex::Male, &t).await.unwrap();

        let outfit_of = |id: i32| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, i16>("SELECT outfit_id FROM players WHERE id = $1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };

        assert_eq!(outfit_of(female).await, t.outfit_id_female);
        assert_eq!(outfit_of(male).await, t.outfit_id_male);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rejects_a_duplicate_name_case_insensitively(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let t = template();
        create(&pool, account_id, "Rizael", Vocation::Knight, Sex::Male, &t).await.unwrap();

        let err = create(&pool, account_id, "RIZAEL", Vocation::Druid, Sex::Female, &t)
            .await
            .unwrap_err();

        assert!(
            matches!(err, AppError::Validation(_)),
            "a taken name must be a validation error, not a 500; got {err:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_deleted_name_stays_reserved(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let t = template();
        let id = create(&pool, account_id, "Rizael", Vocation::Knight, Sex::Male, &t).await.unwrap();

        soft_delete(&pool, id, account_id).await.unwrap();

        let err = create(&pool, account_id, "Rizael", Vocation::Druid, Sex::Female, &t)
            .await
            .unwrap_err();

        assert!(
            matches!(err, AppError::Validation(_)),
            "soft delete must keep the name reserved — the row still exists"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_online_character_cannot_be_soft_deleted(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let id = create(&pool, account_id, "Rizael", Vocation::Knight, Sex::Male, &template())
            .await
            .unwrap();

        sqlx::query("INSERT INTO online_players (character_id) VALUES ($1)")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        assert!(
            matches!(soft_delete(&pool, id, account_id).await, Err(AppError::NotFound)),
            "the UPDATE itself must refuse an online character — a handler pre-check \
             alone races with a login"
        );
        assert_eq!(list_for_account(&pool, account_id).await.unwrap().len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn listing_excludes_deleted_characters(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let t = template();
        let kept = create(&pool, account_id, "Rizael", Vocation::Knight, Sex::Male, &t).await.unwrap();
        let gone = create(&pool, account_id, "Elyra", Vocation::Druid, Sex::Female, &t).await.unwrap();

        soft_delete(&pool, gone, account_id).await.unwrap();

        let listed = list_for_account(&pool, account_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, kept);
        assert_eq!(listed[0].level, 1);
        assert!(!listed[0].online);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn listing_reports_online_status(pool: PgPool) {
        let account_id = an_account(&pool, "player@example.com").await;
        let id = create(&pool, account_id, "Rizael", Vocation::Knight, Sex::Male, &template())
            .await
            .unwrap();

        sqlx::query("INSERT INTO online_players (character_id) VALUES ($1)")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        assert!(list_for_account(&pool, account_id).await.unwrap()[0].online);
        assert!(is_online(&pool, id).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn another_account_cannot_see_or_delete_a_character(pool: PgPool) {
        let owner = an_account(&pool, "owner@example.com").await;
        let stranger = an_account(&pool, "stranger@example.com").await;
        let id = create(&pool, owner, "Rizael", Vocation::Knight, Sex::Male, &template())
            .await
            .unwrap();

        assert!(!belongs_to_account(&pool, id, stranger).await.unwrap());
        assert!(matches!(
            soft_delete(&pool, id, stranger).await,
            Err(AppError::NotFound)
        ));
        assert!(
            belongs_to_account(&pool, id, owner).await.unwrap(),
            "the failed deletion must not have affected the real owner's character"
        );
    }
}
