use anyhow::{Result, anyhow};
use arc_swap::ArcSwap;
use std::collections::binary_heap::BinaryHeap;

use std::sync::Arc;
use std::time::Duration;
use strum::Display;
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
use crate::entities::items::{ItemGuid, ItemRef};
use crate::entities::map::GameMap;
use crate::entities::position::{Direction, ItemPlacement, Position};
use crate::game::creature_behavior::CreatureAction;
use crate::game::events::BroadcastMessage;
use crate::game::item_multi_action::UseTarget;
use crate::game::random::Rolls;
use crate::game::{
    Tick, TickCtx, chat, combat, events, item_action, item_movement, item_multi_action, movement,
    targeting,
};

#[derive(Debug, Display)]
pub enum WorldCommand {
    SpawnPlayer {
        player: Agent,
        session: SessionActorHandle,
        tx: oneshot::Sender<(AgentKey, MessageRouterGuard)>,
    },
    Walk {
        agent_key: AgentKey,
        direction: Direction,
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
        target: UseTarget,
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
        agent_key: AgentKey,
        target: Option<AgentKey>,
        seq: u32,
    },
}

impl WorldCommand {
    pub fn from_creature_action(action: CreatureAction) -> Self {
        match action {
            CreatureAction::Say { agent_key, message } => WorldCommand::Say { agent_key, message },
            CreatureAction::SetTarget { agent_key, target } => WorldCommand::SetTarget {
                agent_key,
                target,
                seq: 0,
            },
            CreatureAction::Walk {
                agent_key,
                direction,
            } => WorldCommand::Walk {
                agent_key,
                direction,
            },
        }
    }
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
    shared_map: Arc<ArcSwap<GameMap>>,
    tick: Tick,
    tick_duration: Duration,
    tick_tx: watch::Sender<Tick>,
    roll: Rolls,
}

impl WorldActor {
    pub fn start(
        map: GameMap,
        shared_map: Arc<ArcSwap<GameMap>>,
        message_router: MessageRouterActorHandle,
        seed: u64,
    ) -> (WorldActorHandle, watch::Receiver<Tick>) {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let (tick_tx, tick_rx) = watch::channel(0);

        let actor = Self {
            rx,
            message_router,
            command_queue: BinaryHeap::with_capacity(CONFIG.max_queue_size),
            map,
            shared_map,
            tick: 0,
            tick_duration: CONFIG.tick_duration,
            tick_tx,
            roll: Rolls::new(seed),
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

            let mut broadcast_messages: Vec<BroadcastMessage> = Vec::new();

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
                    self.handle_command(scheduled.command, &mut broadcast_messages);
                } else {
                    break;
                }
            }

            self.drive_combat(&mut broadcast_messages);

            self.end_tick(broadcast_messages).await;

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

    async fn end_tick(&mut self, mut broadcast_messages: Vec<BroadcastMessage>) {
        events::dedupe_refreshes(&mut broadcast_messages);

        let snapshot = self.map.clone();
        self.shared_map.store(Arc::new(snapshot));
        let _ = self.tick_tx.send(self.tick);
        self.message_router.broadcast(broadcast_messages).await;
    }

    /// Lends the tick's five writable things to `game/` functions as one `TickCtx`, then queues
    /// whatever they scheduled. The queue cannot be touched while the context is alive — it
    /// borrows `self` — so the scheduled commands land in a local first.
    fn with_ctx<T>(
        &mut self,
        broadcast_messages: &mut Vec<BroadcastMessage>,
        f: impl FnOnce(&mut TickCtx) -> T,
    ) -> T {
        let mut scheduled = Vec::new();
        let result = f(&mut TickCtx {
            map: &mut self.map,
            events: broadcast_messages,
            scheduled: &mut scheduled,
            roll: &mut self.roll,
            tick: self.tick,
        });
        for cmd in scheduled {
            self.command_queue.push(cmd);
        }
        result
    }

    fn drive_combat(&mut self, broadcast_messages: &mut Vec<BroadcastMessage>) {
        let with_targets: Vec<AgentKey> = self
            .map
            .iter_agents()
            .filter(|(_, agent)| agent.target().is_some())
            .map(|(key, _)| key)
            .collect();

        self.with_ctx(broadcast_messages, |ctx| {
            for agent_key in with_targets {
                if targeting::drop_unreachable_target(ctx, agent_key) {
                    continue;
                }
                let Some(plan) = combat::plan_auto_attack(ctx.map, agent_key, ctx.roll, ctx.tick)
                else {
                    continue;
                };
                combat::execute_attack(ctx, plan);
            }
        });
    }

    fn handle_command(
        &mut self,
        command: WorldCommand,
        broadcast_messages: &mut Vec<BroadcastMessage>,
    ) {
        info!("Executing command: {}", command);
        let result: Result<()> = match command {
            WorldCommand::SpawnPlayer {
                player,
                session,
                tx,
            } => self.spawn_player(player, session, tx, broadcast_messages),
            WorldCommand::Walk {
                direction,
                agent_key,
            } => {
                self.with_ctx(broadcast_messages, |ctx| {
                    movement::walk(ctx, direction, agent_key)
                });
                Ok(())
            }
            WorldCommand::MoveItem {
                agent,
                source,
                amount,
                to,
                target_container,
            } => {
                self.with_ctx(broadcast_messages, |ctx| {
                    item_movement::move_item(ctx, agent, source, amount, to, target_container)
                });
                Ok(())
            }
            WorldCommand::UseItem { agent, item } => {
                self.with_ctx(broadcast_messages, |ctx| {
                    item_action::use_item(ctx, agent, item)
                });
                Ok(())
            }
            WorldCommand::UseItemWith {
                agent,
                source,
                target,
            } => {
                self.with_ctx(broadcast_messages, |ctx| {
                    item_multi_action::use_item_with(ctx, agent, source, target)
                });
                Ok(())
            }
            WorldCommand::ChangeDirection { agent, facing } => {
                self.with_ctx(broadcast_messages, |ctx| {
                    movement::change_direction(ctx, agent, facing)
                });
                Ok(())
            }
            WorldCommand::SetTarget {
                agent_key,
                target,
                seq,
            } => {
                self.with_ctx(broadcast_messages, |ctx| {
                    targeting::set_target(ctx, agent_key, target, seq)
                });
                Ok(())
            }
            WorldCommand::DespawnPlayer { agent_key, .. } => {
                if let Some((_, position)) = self.map.remove_agent(agent_key) {
                    info!("Player {:?} despawned after disconnect", agent_key);
                    broadcast_messages.push(BroadcastMessage::AgentDespawned {
                        agent_key,
                        snapshot: None,
                        position,
                    });
                }
                Ok(())
            }
            WorldCommand::SpawnCreature {
                kind,
                position,
                spawning,
                slot_idx,
            } => {
                let agent = Agent::from_creature_kind(kind.clone(), position.clone());
                match self.map.insert_agent(agent, &position) {
                    Ok(agent_key) => {
                        broadcast_messages.push(BroadcastMessage::PlayerSpawned {
                            agent_key,
                            position: position.clone(),
                        });
                        if let Err(e) = spawning.creature_spawned(slot_idx, agent_key) {
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
                self.with_ctx(broadcast_messages, |ctx| item_action::decay_item(ctx, item));
                Ok(())
            }
            WorldCommand::Say { agent_key, message } => {
                self.with_ctx(broadcast_messages, |ctx| chat::say(ctx, agent_key, message));
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
        broadcast_messages.push(BroadcastMessage::AgentDespawned {
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
        let origin = agent.get_origin().clone();
        let position = player.last_logout_position().clone();

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
    use crate::actors::message_router::MessageRouterCommand;
    use crate::entities::combat::{CombatDamage, CombatElement};
    use crate::entities::inventory::InventorySlot;
    use crate::entities::map::MapTile;
    use crate::persistence::test_fixtures::{
        a_player_with_a_full_backpack, a_test_creature, a_test_snapshot,
    };

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
            shared_map: Arc::new(ArcSwap::from_pointee(GameMap::new())),
            tick: 0,
            tick_duration: Duration::from_millis(50),
            tick_tx,
            roll: Rolls::new(1),
        }
    }

    /// Pins the call site rather than the collapsing itself — `dedupe_refreshes` has its
    /// own tests in `game::events`. Verified by hand: dropping the call from `end_tick`
    /// makes this fail with `left: 3, right: 2`.
    #[tokio::test]
    async fn the_tick_hands_the_router_one_refresh_per_tile() {
        let mut actor = a_test_world_actor(GameMap::new());
        let (message_router, mut router_rx) = MessageRouterActorHandle::for_test();
        actor.message_router = message_router;
        let position = Position::new(10, 10, 7);

        actor
            .end_tick(vec![
                BroadcastMessage::TileChanged {
                    position: position.clone(),
                },
                BroadcastMessage::DamageTaken {
                    agent_key: AgentKey::default(),
                    position: position.clone(),
                    blood_type: None,
                    damage: CombatDamage {
                        element: CombatElement::Physical,
                        value: 5,
                        blocked_shield: false,
                        blocked_armor: false,
                    },
                },
                BroadcastMessage::TileChanged {
                    position: position.clone(),
                },
            ])
            .await;

        let Some(MessageRouterCommand::Broadcast { messages }) = router_rx.recv().await else {
            panic!("the tick did not broadcast");
        };
        assert_eq!(messages.len(), 2, "{messages:?}");
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

        actor.handle_command(
            WorldCommand::SetTarget {
                agent_key: attacker,
                target: Some(victim),
                seq: 0,
            },
            &mut broadcasts,
        );

        assert_eq!(
            actor.map.get_agent(attacker).unwrap().target(),
            Some(victim)
        );
        assert!(
            broadcasts.is_empty(),
            "a successful set is silent; the client already applied it"
        );
    }

    /// The walk applies before the sweep in the same tick, so the drop lands with
    /// the movement rather than a tick behind it.
    #[tokio::test]
    async fn a_target_out_of_view_is_dropped_by_the_loop() {
        let here = Position::new(100, 100, 7);
        let far = Position::new(120, 100, 7);
        let mut map = GameMap::new();
        map.insert_tile(here.clone(), MapTile::new());
        map.insert_tile(far.clone(), MapTile::new());
        let attacker = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &here)
            .unwrap();
        let victim = map
            .insert_agent(Agent::from_player(a_test_snapshot(2, 1)), &far)
            .unwrap();
        map.get_agent_mut(attacker)
            .unwrap()
            .set_target(Some(victim), 4);

        let mut actor = a_test_world_actor(map);
        let mut broadcasts = Vec::new();

        actor.drive_combat(&mut broadcasts);

        assert_eq!(actor.map.get_agent(attacker).unwrap().target(), None);
        assert!(matches!(
            broadcasts.as_slice(),
            [BroadcastMessage::AgentLostTarget { agent_key, seq: 4 }]
                if *agent_key == attacker
        ));
    }

    #[tokio::test]
    async fn a_reachable_target_is_not_dropped_by_the_loop() {
        let here = Position::new(100, 100, 7);
        let next = Position::new(101, 100, 7);
        let mut map = GameMap::new();
        map.insert_tile(here.clone(), MapTile::new());
        map.insert_tile(next.clone(), MapTile::new());
        let attacker = map
            .insert_agent(Agent::from_player(a_test_snapshot(1, 1)), &here)
            .unwrap();
        let victim = map
            .insert_agent(Agent::from_player(a_test_snapshot(2, 1)), &next)
            .unwrap();
        map.get_agent_mut(attacker)
            .unwrap()
            .set_target(Some(victim), 4);

        let mut actor = a_test_world_actor(map);
        let mut broadcasts = Vec::new();

        actor.drive_combat(&mut broadcasts);

        assert_eq!(
            actor.map.get_agent(attacker).unwrap().target(),
            Some(victim)
        );
        assert!(
            !broadcasts
                .iter()
                .any(|m| matches!(m, BroadcastMessage::AgentLostTarget { .. }))
        );
    }

    /// The behavioural claim of the whole change: a creature killed earlier in the pass
    /// does not get its swing. Insertion order fixes who goes first — `iter_agents` walks
    /// slots in index order — so `killer` is asked before `victim`.
    #[tokio::test]
    async fn a_creature_killed_this_pass_does_not_swing_back() {
        let a = Position::new(5, 5, 7);
        let b = Position::new(6, 5, 7);
        let mut map = GameMap::new();
        map.insert_tile(a.clone(), MapTile::new());
        map.insert_tile(b.clone(), MapTile::new());
        let killer = map
            .insert_agent(a_test_creature("Killer", 100, (5, 5)), &a)
            .unwrap();
        let victim = map
            .insert_agent(a_test_creature("Victim", 1, (7, 7)), &b)
            .unwrap();
        map.get_agent_mut(killer)
            .unwrap()
            .set_target(Some(victim), 0);
        map.get_agent_mut(victim)
            .unwrap()
            .set_target(Some(killer), 0);

        let mut actor = a_test_world_actor(map);
        let mut broadcasts = Vec::new();

        actor.drive_combat(&mut broadcasts);

        assert!(actor.map.get_agent(victim).is_none());
        assert_eq!(actor.map.get_agent(killer).unwrap().life().current, 100);
    }

    /// Covers the publish seam — `shared_map.store(Arc::new(self.map.clone()))` against a
    /// real `WorldActor` and a real `ArcSwap` — not a full tick. No `WorldCommand` is
    /// dispatched; `entities/map.rs`'s tests cover the clone itself.
    #[tokio::test]
    async fn a_published_snapshot_does_not_see_later_inventory_writes() {
        let pos = Position::new(5, 5, 7);
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        let key = map
            .insert_agent(
                Agent::from_player(a_player_with_a_full_backpack(1, 1)),
                &pos,
            )
            .unwrap();

        let mut actor = a_test_world_actor(map);
        actor.shared_map.store(Arc::new(actor.map.clone()));
        let published = actor.shared_map.load_full();

        actor
            .map
            .get_player_mut(key)
            .unwrap()
            .inventory_mut()
            .take_slot(&InventorySlot::Backpack);

        assert!(
            published
                .get_player(key)
                .unwrap()
                .inventory()
                .get(&InventorySlot::Backpack)
                .is_some()
        );
        assert!(
            actor
                .map
                .get_player(key)
                .unwrap()
                .inventory()
                .get(&InventorySlot::Backpack)
                .is_none()
        );
    }
}
