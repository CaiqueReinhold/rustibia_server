use anyhow::{anyhow, Result};
use arc_swap::ArcSwap;
use std::collections::binary_heap::BinaryHeap;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tokio::{
    select,
    sync::{broadcast, mpsc},
};
use tracing::{debug, error, info, warn};

use super::{session::SessionCommand, ActorHandle};
use crate::config::CONFIG;
use crate::entities::agent::{Agent, AgentKey, Facing};
use crate::entities::items::{ItemConfig, ItemGuid, ItemId};
use crate::entities::map::GameMap;
use crate::entities::position::{Direction, ItemPlacement, Position};
use crate::game::events::BroadcastMessage;
use crate::game::{item_action, item_movement, movement, Tick};

#[derive(Clone, Debug)]
pub enum WorldCommand {
    SpawnPlayer {
        player: Agent,
        session: ActorHandle<SessionCommand>,
    },
    Walk {
        direction: Direction,
        actor: AgentKey,
    },
    MoveItem {
        agent: AgentKey,
        from: ItemPlacement,
        item_guid: ItemGuid,
        amount: u8,
        to: ItemPlacement,
        target_container: Option<ItemGuid>,
    },
    UseItem {
        agent: AgentKey,
        guid: ItemGuid,
        placement: ItemPlacement,
    },
    ChangeDirection {
        agent: AgentKey,
        facing: Facing,
    },
    DespawnPlayer {
        agent_key: AgentKey,
        delay_ticks: Tick,
    },
    SpawnAgent {
        agent: Agent,
    },
}

#[derive(Debug)]
pub struct ScheduledCommand {
    at_tick: Tick,
    command: WorldCommand,
}

impl PartialEq for ScheduledCommand {
    fn eq(&self, other: &Self) -> bool {
        self.at_tick == other.at_tick
    }
}

impl Eq for ScheduledCommand {}

impl PartialOrd for ScheduledCommand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledCommand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.at_tick > other.at_tick {
            std::cmp::Ordering::Less
        } else if self.at_tick < other.at_tick {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }
}

pub struct WorldActor {
    rx: mpsc::Receiver<WorldCommand>,
    btx: broadcast::Sender<BroadcastMessage>,
    command_queue: BinaryHeap<ScheduledCommand>,
    map: GameMap,
    item_configs: HashMap<ItemId, Arc<ItemConfig>>,
    shared_map: Arc<ArcSwap<GameMap>>,
    tick: Tick,
    tick_duration: Duration,
    broadcast_messages: VecDeque<BroadcastMessage>,
}

impl WorldActor {
    pub fn start(
        map: GameMap,
        item_configs: HashMap<ItemId, Arc<ItemConfig>>,
        shared_map: Arc<ArcSwap<GameMap>>,
    ) -> (
        ActorHandle<WorldCommand>,
        broadcast::Receiver<BroadcastMessage>,
    ) {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let (btx, brx) = broadcast::channel(CONFIG.max_buffered_messages);

        let actor = Self {
            rx,
            btx,
            command_queue: BinaryHeap::with_capacity(CONFIG.max_queue_size),
            map,
            item_configs,
            shared_map,
            tick: 0,
            tick_duration: CONFIG.tick_duration,
            broadcast_messages: VecDeque::new(),
        };

        tokio::spawn(actor.run());

        (ActorHandle { tx }, brx)
    }

    pub async fn run(mut self) {
        let mut ticker = time::interval(self.tick_duration);

        info!("Starting world loop");
        loop {
            debug!("World: receiving messages");
            loop {
                select! {
                    biased;
                    _ = ticker.tick() => {
                        break
                    },
                    Some(cmd) = self.rx.recv() => {
                        let at_tick = if let WorldCommand::DespawnPlayer { delay_ticks, .. } = &cmd {
                            self.tick + delay_ticks
                        } else {
                            self.tick + 1
                        };
                        self.command_queue.push(ScheduledCommand { at_tick, command: cmd });
                    }
                }
            }

            let tick_start = time::Instant::now();
            self.tick += 1;
            debug!("World: starting tick {}", self.tick);

            if !self.command_queue.is_empty() {
                info!(
                    "Starting tick {} with {} commands",
                    self.tick,
                    self.command_queue.len()
                );
            }

            while let Some(scheduled) = self.command_queue.peek() {
                if scheduled.at_tick <= self.tick {
                    let scheduled = self.command_queue.pop().unwrap();
                    self.handle_command(scheduled.command).await;
                } else {
                    break;
                }
            }

            self.shared_map.store(Arc::new(self.map.clone()));
            while let Some(msg) = self.broadcast_messages.pop_back() {
                info!("World broadcast: {:?}", msg);
                if let Err(e) = self.btx.send(msg) {
                    debug!("No broadcast receivers: {e}");
                }
            }

            let elapsed = tick_start.elapsed();
            debug!("Tick {} took {} ms", self.tick, elapsed.as_millis());
            if elapsed > self.tick_duration {
                warn!(
                    "Tick {} overran budget by {:?}",
                    self.tick,
                    elapsed - self.tick_duration
                );
            }
        }
    }

    fn add_broadcast(&mut self, msg: BroadcastMessage) {
        self.broadcast_messages.push_front(msg);
    }

    fn apply_broadcasts(&mut self, msgs: Vec<BroadcastMessage>) {
        for msg in msgs {
            self.add_broadcast(msg);
        }
    }

    async fn handle_command(&mut self, command: WorldCommand) {
        info!("{:?}", command);
        let result: Result<()> = match command {
            WorldCommand::SpawnPlayer { player, session } => {
                self.spawn_player(player, session).await
            }
            WorldCommand::Walk { direction, actor } => {
                movement::walk(&mut self.map, self.tick, direction, actor)
                    .map(|msgs| self.apply_broadcasts(msgs))
            }
            WorldCommand::MoveItem {
                agent,
                from,
                item_guid,
                amount,
                to,
                target_container,
            } => item_movement::move_item(
                &mut self.map,
                agent,
                from,
                item_guid,
                amount,
                to,
                target_container,
            )
            .map(|msgs| self.apply_broadcasts(msgs)),
            WorldCommand::UseItem {
                agent,
                guid,
                placement,
            } => {
                let msgs = item_action::use_item(
                    &mut self.map,
                    &self.item_configs,
                    agent,
                    guid,
                    placement,
                    self.tick,
                );
                self.apply_broadcasts(msgs);
                Ok(())
            }
            WorldCommand::ChangeDirection { agent, facing } => {
                let msgs = movement::change_direction(&mut self.map, agent, facing);
                self.apply_broadcasts(msgs);
                Ok(())
            }
            WorldCommand::DespawnPlayer { agent_key, .. } => {
                self.map.remove_agent(agent_key);
                info!("Player {:?} despawned after disconnect", agent_key);
                self.add_broadcast(BroadcastMessage::PlayerDespawned { agent_key });
                Ok(())
            }
            WorldCommand::SpawnAgent { agent } => {
                let pos = Position::new(1029, 1028, 7);
                if let Ok(agent_key) = self.map.insert_agent(agent, &pos) {
                    self.add_broadcast(BroadcastMessage::PlayerSpawned {
                        agent_key,
                        position: pos,
                    });
                    Ok(())
                } else {
                    Err(anyhow!("Failed to spawn agent at {:?}", pos))
                }
            }
        };
        if let Err(e) = result {
            error!("Error on apply command: {e}");
        }
    }

    async fn spawn_player(
        &mut self,
        agent: Agent,
        session: ActorHandle<SessionCommand>,
    ) -> Result<()> {
        let player = agent
            .get_player()
            .ok_or(anyhow::anyhow!("agent {:?} is not a player", agent))?;
        let origin = player.origin.clone();
        let position = player.position.clone();

        let agent_key = self
            .map
            .insert_agent(agent.clone(), &position)
            .or_else(|_| self.map.insert_agent(agent, &origin))?;

        let Some(spawn_pos) = self.map.agent_position(agent_key).cloned() else {
            session
                .send(SessionCommand::PlayerSpawnResult(None))
                .await?;
            return Ok(());
        };

        if let Err(e) = session
            .send(SessionCommand::PlayerSpawnResult(Some(agent_key)))
            .await
        {
            self.map.remove_agent(agent_key);
            return Err(e.into());
        }

        self.add_broadcast(BroadcastMessage::PlayerSpawned {
            agent_key,
            position: spawn_pos,
        });

        // testing
        if self.tick < 1000 {
            let creature = Agent::new_creature();
            self.command_queue.push(ScheduledCommand {
                at_tick: self.tick + 500,
                command: WorldCommand::SpawnAgent { agent: creature },
            });
        }

        Ok(())
    }
}
