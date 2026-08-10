use sqlx::PgPool;

use crate::config::SiteConfig;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: SiteConfig,
}
