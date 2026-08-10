mod api;
mod auth;
mod config;
mod db;
mod domain;
mod state;
mod error;
mod template;
mod web;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::{Router, routing::get};
use sqlx::postgres::PgPoolOptions;
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tracing::info;

use crate::{config::SiteConfig, state::AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = SiteConfig::load("config.yaml").context("loading config.yaml")?;

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://rustibia:rustibia@localhost:5432/rustibia".to_string());
    let bind_address =
        std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .context("connecting to Postgres")?;

    sqlx::migrate!().run(&pool).await.context("running migrations")?;
    info!("migrations applied");

    let state = AppState { pool, config };

    // 5 requests burst, refilling one every 2 seconds, keyed by peer IP.
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(2)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(
            web::credential_router()
                .layer(GovernorLayer { config: governor_config.clone() }),
        )
        .merge(web::account_router())
        .merge(web::public_router())
        .nest(
            "/api",
            api::router().layer(GovernorLayer { config: governor_config }),
        )
        .nest_service("/static", tower_http::services::ServeDir::new("static"))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    info!("listening on {bind_address}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
