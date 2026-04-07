#![allow(dead_code)]
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::broadcast::Receiver;
use tracing::info;

mod actors;
mod config;
mod constants;
mod entities;
mod local_id;
mod messages;
mod network;
mod persistence;

use config::CONFIG;

use arc_swap::ArcSwap;

use crate::{
    actors::{
        world::{BroadcastMessage, WorldActor, WorldCommand},
        ActorHandle,
    },
    entities::map::GameMap,
    persistence::player::PlayerRepository,
};

pub struct Context {
    player_repo: Arc<PlayerRepository>,
    world: ActorHandle<WorldCommand>,
    broadcast_receiver: Receiver<BroadcastMessage>,
    shared_map: Arc<ArcSwap<GameMap>>,
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let items = persistence::items::load_items(&CONFIG.items_file_path).unwrap();
    let map = persistence::map::load_map(&CONFIG.map_file_path, &items).unwrap();
    let shared_map = Arc::new(ArcSwap::from_pointee(map.clone()));
    let (world, broadcast_receiver) = WorldActor::start(map, shared_map.clone());

    let context = Context {
        player_repo: Arc::new(PlayerRepository::new()),
        world,
        broadcast_receiver,
        shared_map,
    };

    let listener = network::Listener::bind(CONFIG.bind_address).await?;
    info!("Listening on {}", CONFIG.bind_address);
    listener.listen(context).await;

    Ok(())
}
