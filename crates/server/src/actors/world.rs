use anyhow::{Result, anyhow};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::collections::binary_heap::BinaryHeap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, watch};
use tokio::time;
use tokio::{select, sync::mpsc};
use tracing::{debug, error, info, warn};

use crate::actors::message_router::{MessageRouterActorHandle, MessageRouterGuard};
use crate::actors::session::SessionActorHandle;
use crate::actors::spawning::SpawningActorHandle;
use crate::config::CONFIG;
use crate::entities::agent::{Agent, AgentKey, Facing};
use crate::entities::creature::CreatureKind;
use crate::entities::items::{ItemConfig, ItemGuid, ItemId, ItemRef};
use crate::entities::map::GameMap;
use crate::entities::position::{Direction, ItemPlacement, Position};
use crate::game::events::BroadcastMessage;
use crate::game::{Tick, chat, item_action, item_movement, item_multi_action, movement, targeting};

#[derive(Debug)]
pub enum WorldCommand {
    SpawnPlayer {
        player: Agent,
        session: SessionActorHandle,
        tx: oneshot::Sender<(AgentKey, MessageRouterGuard)>,
    },
    Walk {
        direction: Direction,
        actor: AgentKey,
    },
    MoveItem {
        agent: AgentKey,
        source: ItemRef,
        amount: u8,
        to: ItemPlacement,
        target_container: Option<ItemGuid>,
    },
    UseItem {
        agent: AgentKey,
        item: ItemRef,
    },
    UseItemWith {
        agent: AgentKey,
        source: ItemRef,
        target: ItemRef,
    },
    ChangeDirection {
        agent: AgentKey,
        facing: Facing,
    },
    DespawnPlayer {
        agent_key: AgentKey,
    },
    SpawnCreature {
        kind: Arc<CreatureKind>,
        position: Position,
        spawning: SpawningActorHandle,
        slot_idx: usize,
    },
    RequestLogout {
        agent_key: AgentKey,
    },
    DecayItem {
        item: ItemRef,
    },
    Say {
        agent_key: AgentKey,
        message: String,
    },
    SetTarget {
        agent: AgentKey,
        target: Option<AgentKey>,
    },
}

#[derive(Debug)]
pub struct ScheduledCommand {
    pub at_tick: Tick,
    pub command: WorldCommand,
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

#[derive(Clone, Debug)]
pub struct WorldActorHandle {
    tx: mpsc::Sender<(WorldCommand, Option<Tick>)>,
}

impl WorldActorHandle {
    #[cfg(test)]
    pub fn for_test() -> (Self, mpsc::Receiver<(WorldCommand, Option<Tick>)>) {
        let (tx, rx) = mpsc::channel(64);
        (Self { tx }, rx)
    }

    pub async fn send(&self, command: WorldCommand) {
        let _ = self.tx.send((command, None)).await;
    }

    pub async fn send_delayed(&self, command: WorldCommand, after: Tick) {
        let _ = self.tx.send((command, Some(after))).await;
    }

    pub async fn spawn_player(
        &self,
        player: Agent,
        session: SessionActorHandle,
    ) -> Result<(AgentKey, MessageRouterGuard)> {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .tx
            .send((
                WorldCommand::SpawnPlayer {
                    player,
                    session,
                    tx,
                },
                None,
            ))
            .await;
        Ok(rx.await?)
    }
}

pub struct WorldActor {
    rx: mpsc::Receiver<(WorldCommand, Option<Tick>)>,
    message_router: MessageRouterActorHandle,
    command_queue: BinaryHeap<ScheduledCommand>,
    map: GameMap,
    item_configs: Arc<HashMap<ItemId, Arc<ItemConfig>>>,
    shared_map: Arc<ArcSwap<GameMap>>,
    tick: Tick,
    tick_duration: Duration,
    tick_tx: watch::Sender<Tick>,
}

impl WorldActor {
    pub fn start(
        map: GameMap,
        item_configs: Arc<HashMap<ItemId, Arc<ItemConfig>>>,
        shared_map: Arc<ArcSwap<GameMap>>,
        message_router: MessageRouterActorHandle,
    ) -> (WorldActorHandle, watch::Receiver<Tick>) {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let (tick_tx, tick_rx) = watch::channel(0);

        let actor = Self {
            rx,
            message_router,
            command_queue: BinaryHeap::with_capacity(CONFIG.max_queue_size),
            map,
            item_configs,
            shared_map,
            tick: 0,
            tick_duration: CONFIG.tick_duration,
            tick_tx,
        };

        tokio::spawn(actor.run());

        (WorldActorHandle { tx }, tick_rx)
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
                    Some((command, after)) = self.rx.recv() => {
                        if let Some(after) = after {
                            self.command_queue.push(ScheduledCommand { at_tick: self.tick + after, command });
                        } else {
                            self.command_queue.push(ScheduledCommand { at_tick: self.tick + 1, command });
                        }

                    }
                }
            }

            let tick_start = time::Instant::now();
            self.tick += 1;
            debug!("World: starting tick {}", self.tick);

            let mut broadcast_messages: Vec<BroadcastMessage> =
                Vec::with_capacity(CONFIG.max_queue_size);

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
                    self.handle_command(scheduled.command, &mut broadcast_messages)
                        .await;
                } else {
                    break;
                }
            }

            self.shared_map.store(Arc::new(self.map.clone()));
            let _ = self.tick_tx.send(self.tick);
            self.message_router.broadcast(broadcast_messages).await;

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

    fn apply_commands(&mut self, cmds: Vec<ScheduledCommand>) {
        for cmd in cmds {
            self.command_queue.push(cmd);
        }
    }

    async fn handle_command(
        &mut self,
        command: WorldCommand,
        broadcast_messages: &mut Vec<BroadcastMessage>,
    ) {
        info!("{:?}", command);
        let result: Result<()> = match command {
            WorldCommand::SpawnPlayer {
                player,
                session,
                tx,
            } => self.spawn_player(player, session, tx, broadcast_messages),
            WorldCommand::Walk { direction, actor } => {
                movement::walk(&mut self.map, self.tick, direction, actor)
                    .map(|msgs| broadcast_messages.extend(msgs))
            }
            WorldCommand::MoveItem {
                agent,
                source,
                amount,
                to,
                target_container,
            } => {
                let msgs = item_movement::move_item(
                    &mut self.map,
                    agent,
                    source,
                    amount,
                    to,
                    target_container,
                );
                broadcast_messages.extend(msgs);
                Ok(())
            }
            WorldCommand::UseItem { agent, item } => {
                let (msgs, cmds) = item_action::use_item(
                    &mut self.map,
                    &self.item_configs,
                    agent,
                    item,
                    self.tick,
                );
                broadcast_messages.extend(msgs);
                self.apply_commands(cmds);
                Ok(())
            }
            WorldCommand::UseItemWith {
                agent,
                source,
                target,
            } => {
                let (msgs, cmds) = item_multi_action::use_item_with(
                    &mut self.map,
                    &self.item_configs,
                    agent,
                    source,
                    target,
                    self.tick,
                );
                broadcast_messages.extend(msgs);
                self.apply_commands(cmds);
                Ok(())
            }
            WorldCommand::ChangeDirection { agent, facing } => {
                let msgs = movement::change_direction(&mut self.map, agent, facing);
                broadcast_messages.extend(msgs);
                Ok(())
            }
            WorldCommand::SetTarget { agent, target } => {
                let msgs = targeting::set_target(&mut self.map, agent, target);
                broadcast_messages.extend(msgs);
                Ok(())
            }
            WorldCommand::DespawnPlayer { agent_key, .. } => {
                self.map.remove_agent(agent_key);
                let position = self
                    .map
                    .agent_position(agent_key)
                    .cloned()
                    .unwrap_or_default();
                info!("Player {:?} despawned after disconnect", agent_key);
                broadcast_messages.push(BroadcastMessage::PlayerDespawned {
                    agent_key,
                    snapshot: None,
                    position,
                });
                Ok(())
            }
            WorldCommand::SpawnCreature {
                kind,
                position,
                spawning,
                slot_idx,
            } => {
                let agent = Agent::from_creature_kind(kind.as_ref());
                match self.map.insert_agent(agent, &position) {
                    Ok(agent_key) => {
                        broadcast_messages.push(BroadcastMessage::PlayerSpawned {
                            agent_key,
                            position: position.clone(),
                        });
                        if let Err(e) = spawning.creature_spawned(slot_idx, agent_key).await {
                            error!("Failed to notify SpawningActor of spawn: {e}");
                        }
                        Ok(())
                    }
                    Err(e) => Err(anyhow!(
                        "Failed to spawn creature at {:?}: {:?}",
                        position,
                        e
                    )),
                }
            }
            WorldCommand::RequestLogout { agent_key } => {
                self.handle_request_logout(agent_key, broadcast_messages)
            }
            WorldCommand::DecayItem { item } => {
                let (msgs, commands) =
                    item_action::decay_item(&mut self.map, &self.item_configs, item, self.tick);
                broadcast_messages.extend(msgs);
                self.apply_commands(commands);
                Ok(())
            }
            WorldCommand::Say { agent_key, message } => {
                let msgs = chat::say(&self.map, agent_key, message);
                broadcast_messages.extend(msgs);
                Ok(())
            }
        };
        if let Err(e) = result {
            error!("Error on apply command: {e}");
        }
    }

    fn handle_request_logout(
        &mut self,
        agent_key: AgentKey,
        broadcast_messages: &mut Vec<BroadcastMessage>,
    ) -> Result<()> {
        let Some(agent) = self.map.get_agent(agent_key) else {
            return Ok(());
        };

        if !agent.can_logout(self.tick) {
            let next_tick = agent.next_walk_tick;
            self.command_queue.push(ScheduledCommand {
                at_tick: next_tick,
                command: WorldCommand::RequestLogout { agent_key },
            });
            broadcast_messages.push(BroadcastMessage::LogoutDenied { agent_key });
            return Ok(());
        }

        let position = self.map.agent_position(agent_key).cloned();
        let snapshot = position
            .clone()
            .and_then(|pos| agent.to_snapshot(pos))
            .map(Arc::new);
        self.map.remove_agent(agent_key);
        broadcast_messages.push(BroadcastMessage::PlayerDespawned {
            agent_key,
            snapshot,
            position: position.unwrap_or_default(),
        });
        Ok(())
    }

    fn spawn_player(
        &mut self,
        agent: Agent,
        session: SessionActorHandle,
        tx: oneshot::Sender<(AgentKey, MessageRouterGuard)>,
        broadcast_messages: &mut Vec<BroadcastMessage>,
    ) -> Result<()> {
        let player = agent
            .get_player()
            .ok_or(anyhow!("Agent {:?} is not a player", agent))?;
        let origin = player.origin.clone();
        let position = player.position.clone();

        let agent_key = self
            .map
            .insert_agent(agent.clone(), &position)
            .or_else(|_| self.map.insert_agent(agent, &origin))?;

        let Some(spawn_pos) = self.map.agent_position(agent_key).cloned() else {
            let agent = self.map.remove_agent(agent_key);
            return Err(anyhow!("Player {:?} failed to spawn", agent));
        };

        let guard = self.message_router.subscribe(agent_key, session)?;

        if tx.send((agent_key, guard)).is_err() {
            self.map.remove_agent(agent_key);
            return Err(anyhow!("Failed to return spawned player result"));
        }

        broadcast_messages.push(BroadcastMessage::PlayerSpawned {
            agent_key,
            position: spawn_pos,
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::map::MapTile;
    use crate::persistence::test_fixtures::a_test_snapshot;

    /// Builds a `WorldActor` from bare fields, the same way `SessionActorHandle::for_test`
    /// (session.rs) fabricates a channel-backed handle for tests. The `rx` half of the
    /// command channel and the `message_router`/`tick_tx` handles are never driven; only
    /// `handle_command` is called directly.
    fn a_test_world_actor(map: GameMap) -> WorldActor {
        let (_tx, rx) = mpsc::channel(1);
        let (message_router, _router_rx) = MessageRouterActorHandle::for_test();
        let (tick_tx, _tick_rx) = watch::channel(0);
        WorldActor {
            rx,
            message_router,
            command_queue: BinaryHeap::new(),
            map,
            item_configs: Arc::new(HashMap::new()),
            shared_map: Arc::new(ArcSwap::from_pointee(GameMap::new())),
            tick: 0,
            tick_duration: Duration::from_millis(50),
            tick_tx,
        }
    }

    /// Goes through the real `WorldCommand::SetTarget` dispatch arm in
    /// `handle_command`, not `game::targeting::set_target` directly. Verified by
    /// hand: gutting the dispatch arm to a no-op `Ok(())` makes this test fail on
    /// `assert_eq!(actor.map.get_agent(attacker).unwrap().target(), Some(victim))`
    /// (left: `None`, right: `Some(victim)`); restoring the arm makes it pass again.
    #[tokio::test]
    async fn set_target_command_dispatches_through_handle_command() {
        let mut map = GameMap::new();
        let pos = Position::new(5, 5, 7);
        map.insert_tile(pos.clone(), MapTile::new());
        let attacker = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &pos)
            .unwrap();
        let victim = map
            .insert_agent(Agent::from_player(a_test_snapshot(2, 1)), &pos)
            .unwrap();

        let mut actor = a_test_world_actor(map);
        let mut broadcasts = Vec::new();

        actor
            .handle_command(
                WorldCommand::SetTarget {
                    agent: attacker,
                    target: Some(victim),
                },
                &mut broadcasts,
            )
            .await;

        assert_eq!(
            actor.map.get_agent(attacker).unwrap().target(),
            Some(victim)
        );
        assert!(matches!(
            broadcasts.as_slice(),
            [BroadcastMessage::TargetChanged { agent_key, target: Some(t) }]
                if *agent_key == attacker && *t == victim
        ));
    }
}
