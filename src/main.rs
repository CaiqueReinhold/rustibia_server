#![allow(dead_code)]
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::broadcast::Receiver;
use tracing::info;

mod actors;
mod config;
mod constants;
mod entities;
mod messages;
mod network;
mod persistence;

use config::CONFIG;

use crate::{
    actors::{ActorHandle, BroadcastMessage, WorldCommand},
    persistence::player::PlayerRepository,
};

pub struct Context {
    player_repo: Arc<PlayerRepository>,
    world: ActorHandle<WorldCommand>,
    broadcast_receiver: Receiver<BroadcastMessage>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let items = persistence::items::load_items(&CONFIG.items_file_path).unwrap();
    let map = persistence::map::load_map(&CONFIG.map_file_path, &items).unwrap();
    let (world, broadcast_receiver) = actors::WorldActor::start(map);

    let context = Context {
        player_repo: Arc::new(PlayerRepository::new()),
        world,
        broadcast_receiver,
    };

    let listener = network::Listener::bind(CONFIG.bind_address).await?;
    info!("Listening on {}", CONFIG.bind_address);
    listener.listen(context).await;

    Ok(())
}
