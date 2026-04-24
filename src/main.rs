use std::sync::Arc;

use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::broadcast::Receiver;
use tracing::info;

mod actors;
mod config;
mod constants;
mod entities;
mod game;
mod local_id;
mod messages;
mod network;
mod persistence;

use config::CONFIG;

use arc_swap::ArcSwap;

use crate::{
    actors::{
        persistence::{PersistenceActor, PersistenceActorHandle},
        world::{WorldActor, WorldActorHandle},
    },
    entities::map::GameMap,
    game::{events::BroadcastMessage, game_config::GAME_CONFIG},
    persistence::player::PlayerRepository,
};

pub struct Context {
    player_repo: Arc<PlayerRepository>,
    world: WorldActorHandle,
    broadcast_receiver: Receiver<BroadcastMessage>,
    shared_map: Arc<ArcSwap<GameMap>>,
    persistence: PersistenceActorHandle,
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // access lazy config to make sure it loaded correctly
    let _ = &GAME_CONFIG.action;

    let items = Arc::new(persistence::items::load_items(&CONFIG.items_file_path).unwrap());
    let map = persistence::map::load_map(&CONFIG.map_file_path, &items).unwrap();
    let shared_map = Arc::new(ArcSwap::from_pointee(map.clone()));
    let (world, broadcast_receiver) =
        WorldActor::start(map, Arc::clone(&items), shared_map.clone());

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&CONFIG.database_url)
        .await?;
    sqlx::migrate!().run(&pool).await?;

    let player_repo = Arc::new(PlayerRepository::new(pool, Arc::clone(&items)));
    let persistence = PersistenceActor::start(Arc::clone(&player_repo));

    let context = Context {
        player_repo,
        world,
        broadcast_receiver,
        shared_map,
        persistence,
    };

    let listener = network::Listener::bind(CONFIG.bind_address).await?;
    info!("Listening on {}", CONFIG.bind_address);
    listener.listen(context).await;

    Ok(())
}
