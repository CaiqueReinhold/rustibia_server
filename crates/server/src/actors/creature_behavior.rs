use std::sync::Arc;

use arc_swap::ArcSwap;
use slotmap::Key;
use tokio::sync::watch;
use tracing::{error, info};

use crate::actors::world::{WorldActorHandle, WorldCommand};
use crate::entities::map::GameMap;
use crate::game::Tick;
use crate::game::creature_behavior::{CreatureAction, CreatureBehaviourContext, decide_action};
use crate::game::random::Rolls;

pub struct CreatureBehaviorActor {
    tick_rx: watch::Receiver<Tick>,
    world: WorldActorHandle,
    shared_map: Arc<ArcSwap<GameMap>>,
    seed: u64,
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
            seed,
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
        let map = self.shared_map.load_full();
        let global_seed = self.seed;
        let actions = tokio::task::spawn_blocking(move || {
            map.iter_agents()
                .filter(|(_, a)| a.is_creature())
                .map(|(k, _)| k)
                .flat_map(|agent_key| {
                    let roll = Rolls::stream(global_seed, tick, agent_key.data().as_ffi());
                    decide_action(CreatureBehaviourContext {
                        creature: agent_key,
                        map: &map,
                        roll,
                        world_tick: tick,
                    })
                })
                .collect::<Vec<CreatureAction>>()
        })
        .await;
        match actions {
            Ok(actions) => {
                for action in actions {
                    self.world
                        .send(WorldCommand::from_creature_action(action))
                        .await;
                }
            }
            Err(e) => {
                error!(
                    "Creature behaviour failed to execute for tick {}: {}",
                    tick, e
                );
            }
        }
    }
}
