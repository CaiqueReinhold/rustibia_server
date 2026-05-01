use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::{
    online_registry::OnlineRegistry,
    entities::map::GameMap
};

use self::{
    persistence::PersistenceActorHandle,
    world::WorldActorHandle,
};

pub mod auth;
pub mod connection;
pub mod persistence;
mod player_query;
pub mod session;
pub mod world;

#[derive(Clone)]
pub struct SharedContext {
    pub world: WorldActorHandle,
    pub shared_map: Arc<ArcSwap<GameMap>>,
    pub persistence: PersistenceActorHandle,
    pub online_registry: OnlineRegistry,
}
