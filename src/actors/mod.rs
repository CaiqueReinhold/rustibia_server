use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::{entities::map::GameMap, online_registry::OnlineRegistry};

use self::{persistence::PersistenceActorHandle, world::WorldActorHandle};

pub mod auth;
pub mod connection;
pub mod creature_behavior;
pub mod persistence;
mod player_query;
pub mod session;
pub mod spawning;
pub mod world;

#[derive(Clone)]
pub struct SharedContext {
    pub world: WorldActorHandle,
    pub shared_map: Arc<ArcSwap<GameMap>>,
    pub persistence: PersistenceActorHandle,
    pub online_registry: OnlineRegistry,
}
