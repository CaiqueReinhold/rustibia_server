use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use arc_swap::ArcSwap;
use tokio::select;
use tokio::sync::{mpsc, watch};
use tracing::{error, info};

use crate::actors::world::{WorldActorHandle, WorldCommand};
use crate::config::CONFIG;
use crate::entities::agent::AgentKey;
use crate::entities::creature::{CreatureKind, CreatureKindId};
use crate::entities::map::GameMap;
use crate::game::Tick;
use crate::persistence::spawns::SpawnPoint;

#[derive(Clone, Debug)]
pub enum SpawningCommand {
    CreatureSpawned {
        slot_idx: usize,
        agent_key: AgentKey,
    },
}

#[derive(Clone, Debug)]
pub struct SpawningActorHandle {
    tx: mpsc::Sender<SpawningCommand>,
}

impl SpawningActorHandle {
    pub async fn creature_spawned(
        &self,
        slot_idx: usize,
        agent_key: AgentKey,
    ) -> Result<(), mpsc::error::SendError<SpawningCommand>> {
        self.tx
            .send(SpawningCommand::CreatureSpawned {
                slot_idx,
                agent_key,
            })
            .await
    }
}

#[derive(Debug)]
enum SlotState {
    /// No live creature; ready to spawn at or after `available_at_tick`.
    Empty { available_at_tick: Tick },
    /// SpawnCreature was sent; awaiting `CreatureSpawned` reply.
    Pending,
    /// Creature alive in the world.
    Alive { agent_key: AgentKey },
}

pub struct SpawningActor {
    rx: mpsc::Receiver<SpawningCommand>,
    tick_rx: watch::Receiver<Tick>,
    world: WorldActorHandle,
    shared_map: Arc<ArcSwap<GameMap>>,
    creatures: Arc<HashMap<CreatureKindId, Arc<CreatureKind>>>,
    spawns: Vec<SpawnPoint>,
    states: Vec<SlotState>,
    self_handle: SpawningActorHandle,
}

impl SpawningActor {
    pub fn start(
        spawns: Vec<SpawnPoint>,
        creatures: Arc<HashMap<CreatureKindId, Arc<CreatureKind>>>,
        world: WorldActorHandle,
        shared_map: Arc<ArcSwap<GameMap>>,
        tick_rx: watch::Receiver<Tick>,
    ) -> SpawningActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let self_handle = SpawningActorHandle { tx };

        let states = (0..spawns.len())
            .map(|_| SlotState::Empty {
                available_at_tick: 0,
            })
            .collect();

        let self_handle_clone = self_handle.clone();
        tokio::spawn(async move {
            let actor = Self {
                rx,
                tick_rx,
                world,
                shared_map,
                creatures,
                spawns,
                states,
                self_handle: self_handle_clone,
            };
            actor.run().await;
        });

        self_handle
    }

    async fn run(mut self) {
        info!("SpawningActor started ({} spawn points)", self.spawns.len());
        loop {
            select! { biased;
                Some(cmd) = self.rx.recv() => self.handle_command(cmd),
                Ok(()) = self.tick_rx.changed() => {
                    let tick = *self.tick_rx.borrow();
                    self.process_tick(tick).await;
                },
                else => break,
            }
        }
    }

    fn handle_command(&mut self, cmd: SpawningCommand) {
        match cmd {
            SpawningCommand::CreatureSpawned {
                slot_idx,
                agent_key,
            } => {
                if let Some(state) = self.states.get_mut(slot_idx) {
                    *state = SlotState::Alive { agent_key };
                }
            }
        }
    }

    async fn process_tick(&mut self, tick: Tick) {
        let map = self.shared_map.load();
        for idx in 0..self.spawns.len() {
            self.process_slot(idx, tick, &map).await;
        }
    }

    async fn process_slot(&mut self, idx: usize, tick: Tick, map: &GameMap) {
        let spawn = &self.spawns[idx];
        match &self.states[idx] {
            SlotState::Alive { agent_key } => {
                if map.get_agent(*agent_key).is_none() {
                    self.states[idx] = SlotState::Empty {
                        available_at_tick: tick + spawn.respawn_ticks,
                    };
                }
            }
            SlotState::Empty { available_at_tick } => {
                if tick < *available_at_tick {
                    return;
                }
                let Some(kind) = self.creatures.get(&spawn.kind).cloned() else {
                    error!("Unknown creature kind '{}' for spawn {}", spawn.kind, idx);
                    return;
                };
                let cmd = WorldCommand::SpawnCreature {
                    kind,
                    position: spawn.position.clone(),
                    spawning: self.self_handle.clone(),
                    slot_idx: idx,
                };
                self.world.send(cmd).await;
                self.states[idx] = SlotState::Pending;
            }
            SlotState::Pending => {}
        }
    }
}
