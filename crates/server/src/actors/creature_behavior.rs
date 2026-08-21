use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::sync::watch;
use tracing::info;

use crate::actors::world::{WorldActorHandle, WorldCommand};
use crate::entities::map::GameMap;
use crate::game::Tick;
use crate::game::creature_ai::{CreatureAction, decide_actions};
use crate::game::random::Rolls;

pub struct CreatureBehaviorActor {
    tick_rx: watch::Receiver<Tick>,
    world: WorldActorHandle,
    shared_map: Arc<ArcSwap<GameMap>>,
    roll: Rolls,
}

impl CreatureBehaviorActor {
    pub fn start(
        world: WorldActorHandle,
        shared_map: Arc<ArcSwap<GameMap>>,
        tick_rx: watch::Receiver<Tick>,
        seed: u64,
    ) {
        let actor = Self {
            tick_rx,
            world,
            shared_map,
            roll: Rolls::new(seed),
        };
        tokio::spawn(actor.run());
    }

    async fn run(mut self) {
        info!("CreatureBehaviorActor started");
        while self.tick_rx.changed().await.is_ok() {
            let tick = *self.tick_rx.borrow();
            self.process_tick(tick).await;
        }
    }

    async fn process_tick(&mut self, tick: Tick) {
        let map = self.shared_map.load();
        let actions = decide_actions(&map, tick, &mut self.roll);
        for action in actions {
            let cmd = match action {
                CreatureAction::Walk {
                    agent_key,
                    direction,
                } => WorldCommand::Walk {
                    actor: agent_key,
                    direction,
                },
            };
            self.world.send(cmd).await;
        }
    }
}
