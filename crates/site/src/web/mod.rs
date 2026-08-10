pub mod account;
pub mod admin;
pub mod pages;
pub mod public;

use axum::{Router, routing::get};

use crate::state::AppState;

/// Routes that accept credentials. Kept separate from the public pages that plan 3
/// adds, because Task 13 rate-limits everything in this router.
pub fn credential_router() -> Router<AppState> {
    Router::new()
        .route("/register", get(pages::get_register).post(pages::post_register))
        .route("/login", get(pages::get_login).post(pages::post_login))
        .route("/logout", get(pages::get_logout))
}

/// Authenticated pages. Not rate-limited — a logged-in player browsing their own
/// account must not consume the same budget that protects the login form.
pub fn account_router() -> Router<AppState> {
    Router::new()
        .route("/account", get(account::get_account))
        .route(
            "/account/characters/new",
            get(account::get_character_new).post(account::post_character_new),
        )
        .route(
            "/account/characters/{id}/delete",
            axum::routing::post(account::post_character_delete),
        )
        .route(
            "/account/password",
            get(account::get_password).post(account::post_password),
        )
        .route(
            "/admin/news",
            get(admin::get_admin_news).post(admin::post_admin_news),
        )
}

/// Public pages, reachable without a session. Deliberately NOT rate-limited —
/// browsing news must not consume the budget that protects the login form.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/", get(public::get_news))
        .route("/characters", get(public::get_character_search))
        .route("/characters/{name}", get(public::get_character_detail))
        .route("/online", get(public::get_online))
        .route("/highscores", get(public::get_highscores))
        .route("/download", get(public::get_download))
        .route("/rules", get(public::get_rules))
        .route("/support", get(public::get_support))
}
