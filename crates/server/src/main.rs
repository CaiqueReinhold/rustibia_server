use std::sync::Arc;

use anyhow::{Context as _, Result};
use sqlx::postgres::PgPoolOptions;
use tracing::info;

mod actors;
mod config;
mod constants;
mod entities;
mod game;
mod local_id;
mod messages;
mod network;
mod online_registry;
mod persistence;

use config::CONFIG;

use arc_swap::ArcSwap;

use crate::{
    actors::{
        SharedContext, chat::ChatActor, creature_behavior::CreatureBehaviorActor,
        message_router::MessageRouterActor, persistence::PersistenceActor, spawning::SpawningActor,
        world::WorldActor,
    },
    game::game_config::GAME_CONFIG,
    online_registry::OnlineRegistry,
    persistence::{
        login::{HttpLoginRepository, LoginRepository},
        player::PlayerRepository,
    },
};

/// Generic over the login repository so the HTTP implementation can be swapped for the
/// SQL one without touching the listener; see `persistence::login`.
pub struct Context<L: LoginRepository> {
    login_repo: Arc<L>,
    shared_ctx: SharedContext,
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // access lazy config to make sure it loaded correctly
    let _ = &GAME_CONFIG.action;

    let items = Arc::new(persistence::items::load_items(&CONFIG.items_file_path).unwrap());
    let map = persistence::map::load_map(&CONFIG.map_file_path, &items).unwrap();
    let creatures =
        Arc::new(persistence::creatures::load_creatures(&CONFIG.creatures_file_path).unwrap());
    let spawns = persistence::spawns::load_spawns(&CONFIG.spawns_file_path).unwrap();

    let shared_map = Arc::new(ArcSwap::from_pointee(map.clone()));

    let message_router = MessageRouterActor::start(shared_map.clone());
    let (world, tick_rx) = WorldActor::start(
        map,
        Arc::clone(&items),
        shared_map.clone(),
        message_router.clone(),
    );
    let chat = ChatActor::start(message_router);

    SpawningActor::start(
        spawns,
        Arc::clone(&creatures),
        world.clone(),
        shared_map.clone(),
        tick_rx.clone(),
    );
    CreatureBehaviorActor::start(world.clone(), shared_map.clone(), tick_rx);

    let internal_client = HttpLoginRepository::build_client(
        CONFIG.internal_tls_cert.as_str(),
        CONFIG.internal_tls_key.as_str(),
        CONFIG.internal_tls_ca.as_str(),
    )
    .context(
        "building the internal mTLS client — run `cargo run -p rustibia-certgen` to \
         generate certs/, or point INTERNAL_TLS_CERT/_KEY/_CA at existing ones",
    )?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&CONFIG.database_url)
        .await?;

    let online_repo = Arc::new(persistence::online::OnlineRepository::new(pool.clone()));
    online_repo
        .clear_all()
        .await
        .context("clearing stale online_players rows")?;

    // The pool remains only for saving and online tracking. Login no longer touches it.
    let player_repo = Arc::new(PlayerRepository::new(pool));
    let login_repo = Arc::new(HttpLoginRepository::new(
        &CONFIG.site_internal_url,
        internal_client,
        Arc::clone(&items),
    ));
    let persistence = PersistenceActor::start(Arc::clone(&player_repo), Arc::clone(&online_repo));

    let context = Context {
        login_repo,
        shared_ctx: SharedContext {
            world,
            shared_map,
            persistence: persistence.clone(),
            online_registry: OnlineRegistry::new(persistence),
            chat,
        },
    };

    let listener = network::Listener::bind(CONFIG.bind_address).await?;
    info!("Listening on {}", CONFIG.bind_address);
    listener.listen(context).await;

    Ok(())
}
