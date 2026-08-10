use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{
    db::{news, public},
    error::{AppError, Surface, SurfacedError},
    state::AppState,
    template::HtmlTemplate,
};

/// How many news posts the homepage shows.
const NEWS_ON_HOMEPAGE: i64 = 10;
/// Spec §8: top 100, no pagination.
const HIGHSCORE_LIMIT: i64 = 100;

// Every page struct carries `is_admin` from the start. A later task adds an Admin
// group to `base.html` that reads it; declaring the field now means that task edits
// one template instead of rewriting eight structs. Askama does not mind a field the
// template does not yet reference.

#[derive(Template)]
#[template(path = "news.html")]
pub struct NewsPage {
    pub logged_in: bool,
    pub is_admin: bool,
    pub posts: Vec<news::NewsPost>,
}

#[derive(Template)]
#[template(path = "character_search.html")]
pub struct CharacterSearchPage {
    pub logged_in: bool,
    pub is_admin: bool,
    pub name: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "character_detail.html")]
pub struct CharacterDetailPage {
    pub logged_in: bool,
    pub is_admin: bool,
    pub character: public::CharacterDetail,
    pub created: String,
}

#[derive(Template)]
#[template(path = "online.html")]
pub struct OnlinePage {
    pub logged_in: bool,
    pub is_admin: bool,
    pub characters: Vec<public::OnlineCharacter>,
}

#[derive(Template)]
#[template(path = "highscores.html")]
pub struct HighscoresPage {
    pub logged_in: bool,
    pub is_admin: bool,
    pub entries: Vec<public::HighscoreEntry>,
}

#[derive(Template)]
#[template(path = "download.html")]
pub struct DownloadPage {
    pub logged_in: bool,
    pub is_admin: bool,
}

#[derive(Template)]
#[template(path = "rules.html")]
pub struct RulesPage {
    pub logged_in: bool,
    pub is_admin: bool,
}

#[derive(Template)]
#[template(path = "support.html")]
pub struct SupportPage {
    pub logged_in: bool,
    pub is_admin: bool,
}

fn page_error(err: AppError) -> SurfacedError {
    SurfacedError(Surface::Page, err)
}

pub async fn get_news(State(state): State<AppState>) -> Result<impl IntoResponse, SurfacedError> {
    let posts = news::list_recent(&state.pool, NEWS_ON_HOMEPAGE)
        .await
        .map_err(page_error)?;

    Ok(HtmlTemplate(NewsPage { logged_in: false, is_admin: false, posts }))
}

#[derive(Debug, Deserialize)]
pub struct CharacterQuery {
    pub name: Option<String>,
}

/// Exact-name lookup, redirecting to the detail page on a hit.
///
/// Deliberately not a partial or prefix search: the name column is unique and the
/// point is to reach one character, so a substring search would only serve to
/// enumerate the player base.
pub async fn get_character_search(
    State(state): State<AppState>,
    Query(query): Query<CharacterQuery>,
) -> Result<Response, SurfacedError> {
    let Some(name) = query.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(HtmlTemplate(CharacterSearchPage {
            logged_in: false,
            is_admin: false,
            name: String::new(),
            error: None,
        })
        .into_response());
    };

    match public::find_character_by_name(&state.pool, name).await.map_err(page_error)? {
        Some(character) => Ok(Redirect::to(&format!("/characters/{}", character.name)).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            HtmlTemplate(CharacterSearchPage {
                logged_in: false,
                is_admin: false,
                name: name.to_string(),
                error: Some("Character not found.".to_string()),
            }),
        )
            .into_response()),
    }
}

pub async fn get_character_detail(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, SurfacedError> {
    let Some(character) = public::find_character_by_name(&state.pool, &name)
        .await
        .map_err(page_error)?
    else {
        return Ok((
            StatusCode::NOT_FOUND,
            HtmlTemplate(CharacterSearchPage {
                logged_in: false,
                is_admin: false,
                name,
                error: Some("Character not found.".to_string()),
            }),
        )
            .into_response());
    };

    let created = character
        .created_at
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    Ok(HtmlTemplate(CharacterDetailPage { logged_in: false, is_admin: false, character, created })
        .into_response())
}

pub async fn get_online(State(state): State<AppState>) -> Result<impl IntoResponse, SurfacedError> {
    let characters = public::who_is_online(&state.pool).await.map_err(page_error)?;
    Ok(HtmlTemplate(OnlinePage { logged_in: false, is_admin: false, characters }))
}

pub async fn get_highscores(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, SurfacedError> {
    let entries = public::highscores(&state.pool, HIGHSCORE_LIMIT)
        .await
        .map_err(page_error)?;
    Ok(HtmlTemplate(HighscoresPage { logged_in: false, is_admin: false, entries }))
}

pub async fn get_download() -> impl IntoResponse {
    HtmlTemplate(DownloadPage { logged_in: false, is_admin: false })
}

pub async fn get_rules() -> impl IntoResponse {
    HtmlTemplate(RulesPage { logged_in: false, is_admin: false })
}

pub async fn get_support() -> impl IntoResponse {
    HtmlTemplate(SupportPage { logged_in: false, is_admin: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::SiteConfig,
        db::{accounts::create_account, characters},
        domain::{sex::Sex, vocation::Vocation},
    };
    use axum::{Router, body::Body, http::{Request, header}, routing::get};
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn test_app(pool: PgPool) -> Router {
        let config = SiteConfig::load("config.yaml").unwrap();
        Router::new()
            .route("/", get(get_news))
            .route("/characters", get(get_character_search))
            .route("/characters/{name}", get(get_character_detail))
            .route("/online", get(get_online))
            .route("/highscores", get(get_highscores))
            .route("/download", get(get_download))
            .route("/rules", get(get_rules))
            .route("/support", get(get_support))
            .with_state(AppState { pool, config })
    }

    async fn fetch(app: Router, uri: &str) -> (StatusCode, String, Option<String>) {
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&body).to_string(), location)
    }

    async fn a_character(pool: &PgPool, email: &str, name: &str) -> i32 {
        let template = SiteConfig::load("config.yaml").unwrap().new_character;
        let account = create_account(pool, email, "hunter2hunter2").await.unwrap();
        characters::create(pool, account.id, name, Vocation::Druid, Sex::Female, &template)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_homepage_renders_with_no_news(pool: PgPool) {
        let (status, body, _) = fetch(test_app(pool), "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("No news yet"));
        assert!(body.contains("RUSTIBIA"), "the retro frame must render");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_homepage_shows_a_post(pool: PgPool) {
        let account = create_account(&pool, "admin@example.com", "hunter2hunter2").await.unwrap();
        crate::db::news::create(&pool, "Server is open", "Come and play.", account.id)
            .await
            .unwrap();

        let (status, body, _) = fetch(test_app(pool), "/").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Server is open"));
        assert!(body.contains("Come and play."));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_news_post_cannot_inject_markup(pool: PgPool) {
        let account = create_account(&pool, "admin@example.com", "hunter2hunter2").await.unwrap();
        crate::db::news::create(&pool, "<script>x</script>", "<b>bold</b>", account.id)
            .await
            .unwrap();

        let (_, body, _) = fetch(test_app(pool), "/").await;

        assert!(!body.contains("<script>x</script>"), "askama must escape the title");
        assert!(!body.contains("<b>bold</b>"), "askama must escape the body");
        // askama 0.14 escapes to numeric entities (`&#60;`), not named ones (`&lt;`).
        assert!(body.contains("&#60;script&#62;"), "and render it as text instead");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_search_page_renders_empty(pool: PgPool) {
        let (status, body, _) = fetch(test_app(pool), "/characters").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Search Characters"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn searching_an_existing_name_redirects_to_the_detail_page(pool: PgPool) {
        a_character(&pool, "a@example.com", "Rizael").await;

        let (status, _, location) = fetch(test_app(pool), "/characters?name=rIzAeL").await;

        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(
            location.as_deref(),
            Some("/characters/Rizael"),
            "the redirect must use the stored capitalisation"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn searching_an_unknown_name_says_not_found(pool: PgPool) {
        let (status, body, _) = fetch(test_app(pool), "/characters?name=Nobody").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("Character not found"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_detail_page_shows_the_character(pool: PgPool) {
        a_character(&pool, "a@example.com", "Rizael").await;

        let (status, body, _) = fetch(test_app(pool), "/characters/Rizael").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Rizael"));
        assert!(body.contains("Druid"));
        assert!(body.contains("Female"));
        assert!(!body.contains("has been deleted"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_detail_page_flags_a_deleted_character(pool: PgPool) {
        let id = a_character(&pool, "a@example.com", "Rizael").await;
        sqlx::query("UPDATE players SET deleted_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        let (status, body, _) = fetch(test_app(pool), "/characters/Rizael").await;

        assert_eq!(status, StatusCode::OK, "old links must still resolve");
        assert!(body.contains("has been deleted"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unknown_detail_page_is_404(pool: PgPool) {
        let (status, _, _) = fetch(test_app(pool), "/characters/Nobody").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_online_page_reports_the_count(pool: PgPool) {
        let id = a_character(&pool, "a@example.com", "Rizael").await;
        sqlx::query("INSERT INTO online_players (character_id) VALUES ($1)")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        let (status, body, _) = fetch(test_app(pool), "/online").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Rizael"));
        assert!(body.contains("1</strong> player(s)"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_highscores_page_lists_characters(pool: PgPool) {
        a_character(&pool, "a@example.com", "Rizael").await;

        let (status, body, _) = fetch(test_app(pool), "/highscores").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Rizael"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_static_pages_all_render(pool: PgPool) {
        for uri in ["/download", "/rules", "/support"] {
            let (status, body, _) = fetch(test_app(pool.clone()), uri).await;
            assert_eq!(status, StatusCode::OK, "{uri} must render");
            assert!(body.contains("RUSTIBIA"), "{uri} must use the retro frame");
        }
    }
}
