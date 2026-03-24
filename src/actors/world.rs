use std::collections::binary_heap::BinaryHeap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tokio::{
    select,
    sync::{broadcast, mpsc},
};
use tracing::{debug, error, info, warn};

use arc_swap::ArcSwap;

use super::{session::SessionCommand, ActorHandle};
use crate::config::CONFIG;
use crate::entities::agent::AgentKey;
use crate::entities::map::GameMap;
use crate::entities::player::Player;
use crate::entities::position::{Direction, Position};

pub type Tick = u64;

#[derive(Clone, Debug)]
pub enum WorldCommand {
    SpawnPlayer {
        player: Player,
        session: ActorHandle<SessionCommand>,
    },
    Walk {
        direction: Direction,
        actor: AgentKey,
        session: ActorHandle<SessionCommand>,
    },
    WalkFinished {
        new_position: Position,
        actor: AgentKey,
    },
}

#[derive(Clone, Debug)]
pub enum BroadcastMessage {
    PlayerSpawned {
        agent_key: AgentKey,
        position: Position,
    },
    AgentMoved {
        agent_key: AgentKey,
        direction: Direction,
        from_pos: Position,
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
    shared_map: Arc<ArcSwap<GameMap>>,
    tick: Tick,
    tick_duration: Duration,
}

impl WorldActor {
    pub fn start(
        map: GameMap,
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
            shared_map,
            tick: 0,
            tick_duration: CONFIG.tick_duration,
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
                    _ = ticker.tick() => {
                        break
                    },
                    Some(cmd) = self.rx.recv() => {
                        self.command_queue.push(ScheduledCommand { at_tick: self.tick + 1, command: cmd });
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

            let mut broadcast_messages: Vec<BroadcastMessage> = Vec::new();

            while let Some(scheduled) = self.command_queue.peek() {
                if scheduled.at_tick <= self.tick {
                    let scheduled = self.command_queue.pop().unwrap();
                    self.handle_command(scheduled.command, &mut broadcast_messages)
                        .await;
                } else {
                    break;
                }
            }

            self.shared_map.store(Arc::new(self.map.clone()));
            for msg in broadcast_messages {
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

    async fn handle_command(
        &mut self,
        command: WorldCommand,
        broadcast: &mut Vec<BroadcastMessage>,
    ) {
        info!("{:?}", command);
        let msg = match command {
            WorldCommand::SpawnPlayer { player, session } => {
                self.spawn_player(player, session).await
            }
            WorldCommand::Walk {
                direction,
                actor,
                session,
            } => self.walk(direction, actor, session).await,
            WorldCommand::WalkFinished {
                new_position,
                actor,
            } => self.walk_finished(new_position, actor).await,
        };
        if let Some(message) = msg {
            broadcast.push(message);
        }
    }

    async fn spawn_player(
        &mut self,
        player: Player,
        session: ActorHandle<SessionCommand>,
    ) -> Option<BroadcastMessage> {
        let origin = player.origin.clone();
        let position = player.position.clone();

        let agent_key = self
            .map
            .insert_agent(
                crate::entities::agent::Agent::from_player(player.clone()),
                &position,
            )
            .or_else(|_| {
                self.map
                    .insert_agent(crate::entities::agent::Agent::from_player(player), &origin)
            })
            .ok()?;

        self.map.get_agent_mut(agent_key).unwrap().handle = Some(agent_key);

        let spawn_pos = self.map.agent_position(agent_key)?.clone();

        if session
            .send(SessionCommand::PlayerSpawnResult(Some(agent_key)))
            .await
            .is_err()
        {
            self.map.remove_agent(agent_key);
            return None;
        }

        Some(BroadcastMessage::PlayerSpawned {
            agent_key,
            position: spawn_pos,
        })
    }

    async fn walk(
        &mut self,
        direction: Direction,
        agent_key: AgentKey,
        session: ActorHandle<SessionCommand>,
    ) -> Option<BroadcastMessage> {
        if self.map.get_agent(agent_key).is_none() {
            error!("Message from actor not spawned");
            return None;
        }

        let agent = match self.map.get_agent_mut(agent_key) {
            Some(a) => a,
            None => return None,
        };

        if agent.next_walk_tick > self.tick {
            return None;
        }

        let current_pos = self.map.agent_position(agent_key)?.clone();
        let new_pos = current_pos.clone() + direction.clone();
        let can_move = self.map.can_move(&new_pos, agent_key);
        let tile_speed = self.map.tile_speed(&new_pos);
        let walk_ticks = self
            .map
            .get_agent(agent_key)?
            .calculate_walk_ticks(tile_speed, direction.is_diagonal());
        self.map.get_agent_mut(agent_key)?.next_walk_tick = self.tick + walk_ticks;

        if !can_move {
            let _ = session
                .send(SessionCommand::PlayerPosition(current_pos))
                .await;
            return None;
        }

        let new_pos = current_pos.clone() + direction.clone();
        self.command_queue.push(ScheduledCommand {
            at_tick: self.tick + walk_ticks,
            command: WorldCommand::WalkFinished {
                new_position: new_pos,
                actor: agent_key,
            },
        });
        Some(BroadcastMessage::AgentMoved {
            agent_key,
            direction,
            from_pos: current_pos,
        })
    }

    async fn walk_finished(
        &mut self,
        new_position: Position,
        agent_key: AgentKey,
    ) -> Option<BroadcastMessage> {
        if self.map.get_agent(agent_key).is_none() {
            error!("WalkFinished but agent {:?} is missing", agent_key);
            return None;
        }
        if let Err(e) = self.map.move_agent(agent_key, &new_position) {
            error!("move_agent failed: {:?}", e);
        }
        None
    }
}
