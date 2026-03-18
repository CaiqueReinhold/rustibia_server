use slotmap::SlotMap;
use std::collections::binary_heap::BinaryHeap;
use std::time::Duration;
use tokio::time;
use tokio::{
    select,
    sync::{broadcast, mpsc},
};
use tracing::{debug, error, info, warn};

use super::{session::SessionCommand, ActorHandle};
use crate::config::CONFIG;
use crate::entities::agent::{Agent, AgentKey};
use crate::entities::map::{DoubleBufferedMap, GameMap, Position};
use crate::entities::player::Player;
use crate::messages::Direction;

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
    PlayerMoved {
        agent_key: AgentKey,
        direction: Direction,
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
    map: DoubleBufferedMap,
    agents: SlotMap<AgentKey, Agent>,
    tick: Tick,
    tick_duration: Duration,
}

impl WorldActor {
    pub fn start(
        map: GameMap,
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
            map: DoubleBufferedMap::new(map),
            agents: SlotMap::with_key(),
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
            info!("World: receiving messages");
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
            self.map.swap();
            self.tick += 1;
            info!("World: starting tick {}", self.tick);

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

            let elapsed = tick_start.elapsed();
            info!("Tick {} took {} ms", self.tick, elapsed.as_millis());
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
        debug!("{:?}", command);
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
        let mut position = player.position.clone();
        let origin = player.origin.clone();

        let agent = Agent::from_player(player);
        let agent_key = self.agents.insert(agent);
        let agent = self.agents.get_mut(agent_key).unwrap();
        agent.handle = Some(agent_key);

        let mut failed_to_spawn = false;
        {
            let mut map = self.map.write();
            if map.add_actor(&position, agent_key).is_err() {
                if map.add_actor(&origin, agent_key).is_err() {
                    failed_to_spawn = true;
                }
                position = origin;
            }
        }
        if failed_to_spawn {
            let actor = self.agents.remove(agent_key).unwrap().handle;
            error!("Player failed to spawn");
            if let Err(e) = session.send(SessionCommand::PlayerSpawnResult(None)).await {
                error!("Session for player {:?} closed: {e}", actor);
            }
            return None;
        }

        agent.set_position(position.clone());

        if session
            .send(SessionCommand::PlayerSpawnResult(Some(agent_key)))
            .await
            .is_err()
        {
            self.agents.remove(agent_key);
            return None;
        };

        Some(BroadcastMessage::PlayerSpawned {
            agent_key,
            position,
        })
    }

    async fn walk(
        &mut self,
        direction: Direction,
        agent_key: AgentKey,
        session: ActorHandle<SessionCommand>,
    ) -> Option<BroadcastMessage> {
        let agent = self.agents.get_mut(agent_key);
        if agent.is_none() {
            warn!("Message from actor not spawned");
            let _ = session.send(SessionCommand::Close).await;
            return None;
        }
        let agent = agent.unwrap();

        if agent.next_walk_tick > self.tick {
            return None;
        }

        let new_position = agent.position().clone() + direction.clone();
        let can_move: bool;
        let tile_speed: u8;
        {
            let map = self.map.read();
            can_move = map.can_move(&new_position, agent);
            tile_speed = map.tile_speed(&new_position);
        }

        if !can_move {
            if session
                .send(SessionCommand::PlayerPosition(agent.position().clone()))
                .await
                .is_err()
            {
                warn!("Session closed");
            }
            return None;
        }

        let walk_ticks = agent.calculate_walk_ticks(tile_speed);
        agent.next_walk_tick = self.tick + walk_ticks;

        self.command_queue.push(ScheduledCommand {
            at_tick: self.tick + walk_ticks,
            command: WorldCommand::WalkFinished {
                new_position,
                actor: agent_key,
            },
        });
        Some(BroadcastMessage::PlayerMoved {
            agent_key,
            direction,
        })
    }

    async fn walk_finished(
        &mut self,
        new_position: Position,
        agent_key: AgentKey,
    ) -> Option<BroadcastMessage> {
        let actor = self.agents.get_mut(agent_key);
        if actor.is_none() {
            error!("Walk finsihed but actor is missing {:?}", agent_key);
            return None;
        }
        let actor = actor.unwrap();

        {
            let mut map = self.map.write();
            let _ = map.remove_actor(actor.position(), agent_key);
            let _ = map.add_actor(&new_position, agent_key);
        }
        actor.set_position(new_position);
        None
    }
}
