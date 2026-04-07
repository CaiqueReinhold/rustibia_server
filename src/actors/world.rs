use anyhow::Result;
use arc_swap::ArcSwap;
use std::collections::binary_heap::BinaryHeap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tokio::{
    select,
    sync::{broadcast, mpsc},
};
use tracing::{debug, info, warn};

use super::{session::SessionCommand, ActorHandle};
use crate::config::CONFIG;
use crate::entities::agent::{Agent, AgentKey};
use crate::entities::items::{ItemFlag, ItemGuid};
use crate::entities::map::GameMap;
use crate::entities::player::InventorySlot;
use crate::entities::position::{Direction, ItemPlacement, Position};

pub type Tick = u64;

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
        to_position: Position,
    },
    TileChanged {
        position: Position,
    },
    MoveDenied {
        agent_key: AgentKey,
    },
    MoveAck {
        agent_key: AgentKey,
    },
    OpenContainer {
        agent_key: AgentKey,
        guid: ItemGuid,
        placement: ItemPlacement,
    },
    UseItemAck {
        agent_key: AgentKey,
        success: bool,
    },
    UpdateContainer {
        guid: ItemGuid,
        placement: ItemPlacement,
    },
    PlayerWalkDenied {
        agent_key: AgentKey,
    },
    UpdateInventorySlot {
        agent_key: AgentKey,
        slot: InventorySlot,
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
    broadcast_messages: VecDeque<BroadcastMessage>,
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

    async fn handle_command(&mut self, command: WorldCommand) {
        info!("{:?}", command);
        let result = match command {
            WorldCommand::SpawnPlayer { player, session } => {
                self.spawn_player(player, session).await
            }
            WorldCommand::Walk { direction, actor } => self.walk(direction, actor).await,
            WorldCommand::MoveItem {
                agent,
                from,
                item_guid,
                amount,
                to,
                target_container,
            } => {
                self.move_item(agent, from, item_guid, amount, to, target_container)
                    .await
            }
            WorldCommand::UseItem {
                agent,
                guid,
                placement,
            } => self.use_item(agent, guid, placement).await,
        };
        if let Err(e) = result {
            warn!("Error on apply command: {e}");
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
        Ok(())
    }

    async fn walk(&mut self, direction: Direction, agent_key: AgentKey) -> Result<()> {
        let agent = self
            .map
            .get_agent(agent_key)
            .ok_or(anyhow::anyhow!("agent {:?} not spawned", agent_key))?;
        let Some(current_pos) = self.map.agent_position(agent_key).cloned() else {
            return Err(anyhow::anyhow!("agent {:?} position not found", agent_key));
        };

        if agent.next_walk_tick > self.tick {
            self.add_broadcast(BroadcastMessage::PlayerWalkDenied { agent_key });
            return Ok(());
        }

        let new_pos = current_pos.clone() + direction;
        let can_move = self.map.can_move(&new_pos, agent_key);
        if !can_move {
            self.add_broadcast(BroadcastMessage::PlayerWalkDenied { agent_key });
            return Ok(());
        }
        let Some(tile_friction) = self.map.tile_friction(&new_pos) else {
            self.add_broadcast(BroadcastMessage::PlayerWalkDenied { agent_key });
            return Ok(());
        };

        let walk_ticks = self
            .map
            .get_agent(agent_key)
            .unwrap()
            .calculate_walk_ticks(tile_friction, direction.is_diagonal());
        self.map.get_agent_mut(agent_key).unwrap().next_walk_tick = self.tick + walk_ticks;

        self.map.move_agent(agent_key, &new_pos)?;
        self.add_broadcast(BroadcastMessage::AgentMoved {
            agent_key,
            direction,
            to_position: new_pos,
        });
        Ok(())
    }

    async fn move_item(
        &mut self,
        agent: AgentKey,
        from: ItemPlacement,
        item_guid: ItemGuid,
        amount: u8,
        to: ItemPlacement,
        target_container: Option<ItemGuid>,
    ) -> Result<()> {
        let player_pos = self
            .map
            .agent_position(agent)
            .ok_or(anyhow::anyhow!("agent {:?} position not found", agent))?;

        if let ItemPlacement::Map(pos) = &from {
            if !player_pos.is_adjacent(pos) {
                self.add_broadcast(BroadcastMessage::MoveDenied { agent_key: agent });
                return Ok(());
            }
        }

        let source = self.map.remove_item(&from, item_guid, amount).map(|it| {
            if let Some((parent, _)) = &it.1 {
                (
                    BroadcastMessage::UpdateContainer {
                        guid: parent.clone(),
                        placement: from.clone(),
                    },
                    it,
                )
            } else {
                match &from {
                    ItemPlacement::Map(pos) => (
                        BroadcastMessage::TileChanged {
                            position: pos.clone(),
                        },
                        it,
                    ),
                    ItemPlacement::Inventory(slot, agent_key) => (
                        BroadcastMessage::UpdateInventorySlot {
                            agent_key: *agent_key,
                            slot: *slot,
                        },
                        it,
                    ),
                }
            }
        });

        let Some((source_change, (item, parent))) = source else {
            self.add_broadcast(BroadcastMessage::MoveDenied { agent_key: agent });
            return Ok(());
        };

        let target_change = if let Some(target_container) = target_container.as_ref() {
            if self
                .map
                .add_to_container(&to, target_container, 0, item.clone())
                .is_ok()
            {
                Some(BroadcastMessage::UpdateContainer {
                    guid: target_container.clone(),
                    placement: to.clone(),
                })
            } else {
                None
            }
        } else {
            match &to {
                ItemPlacement::Map(pos) => {
                    if self.map.drop_item(pos, item.clone()).is_ok() {
                        Some(BroadcastMessage::TileChanged {
                            position: pos.clone(),
                        })
                    } else {
                        None
                    }
                }
                ItemPlacement::Inventory(slot, _) => {
                    let current_item = self
                        .map
                        .get_agent_mut(agent)
                        .and_then(|ag| ag.get_player_mut())
                        .and_then(|player| player.inventory.remove(slot));

                    // first, remove the item occupying the slot and place it in the source position
                    if let Some(current_item) = current_item {
                        if let Some((container_guid, container_slot)) = &parent {
                            if self
                                .map
                                .add_to_container(
                                    &from,
                                    container_guid,
                                    *container_slot,
                                    current_item.clone(),
                                )
                                .is_err()
                            {
                                // if it can't fit the container, place on the ground at the player pos
                                let pos = self.map.agent_position(agent).cloned();
                                if let Some(pos) = pos {
                                    let _ = self.map.drop_item(&pos, current_item);
                                }
                            }
                        } else if let ItemPlacement::Map(pos) = &from {
                            let _ = self.map.drop_item(pos, current_item);
                        }
                        // no need to cover ItemPlacement::Iventory as it would be rejected early
                        // on slot type validation
                    }

                    // now
                    let player = self
                        .map
                        .get_agent_mut(agent)
                        .and_then(|ag| ag.get_player_mut());
                    if let Some(player) = player {
                        player.inventory.insert(*slot, item.clone());
                        Some(BroadcastMessage::UpdateInventorySlot {
                            agent_key: agent,
                            slot: *slot,
                        })
                    } else {
                        None
                    }
                }
            }
        };

        let Some(tartget_change) = target_change else {
            // Try to put the item back in the source if the target failed
            if let Some((container_guid, slot)) = &parent {
                let _ = self
                    .map
                    .add_to_container(&from, container_guid, *slot, item);
            } else {
                match &from {
                    ItemPlacement::Map(pos) => {
                        let _ = self.map.drop_item(pos, item);
                    }
                    ItemPlacement::Inventory(slot, _) => {
                        self.map
                            .get_agent_mut(agent)
                            .and_then(|a| a.get_player_mut())
                            .and_then(|player| player.inventory.insert(*slot, item));
                    }
                }
            }

            self.add_broadcast(BroadcastMessage::MoveDenied { agent_key: agent });
            return Ok(());
        };

        self.add_broadcast(BroadcastMessage::MoveAck { agent_key: agent });
        self.add_broadcast(source_change);
        self.add_broadcast(tartget_change);

        Ok(())
    }

    async fn use_item(
        &mut self,
        agent_key: AgentKey,
        guid: ItemGuid,
        placement: ItemPlacement,
    ) -> Result<()> {
        let item = match &placement {
            ItemPlacement::Map(item_pos) => {
                if self
                    .map
                    .agent_position(agent_key)
                    .filter(|player_pos| player_pos.is_adjacent(item_pos))
                    .is_none()
                {
                    self.add_broadcast(BroadcastMessage::UseItemAck {
                        agent_key,
                        success: false,
                    });
                    return Ok(());
                }
                self.map.get_item_by_id(item_pos, &guid)
            }
            ItemPlacement::Inventory(slot, inv_agent_key) => {
                self.map
                    .get_agent(*inv_agent_key)
                    .and_then(|agent| agent.get_player())
                    .and_then(|player| player.inventory.get(slot))
                    .filter(|item| item.guid == guid)
            }
        };
        let Some(item) = item else {
            self.add_broadcast(BroadcastMessage::UseItemAck {
                agent_key,
                success: false,
            });
            return Ok(());
        };

        if item.config.has_flag(ItemFlag::Container) {
            self.add_broadcast(BroadcastMessage::UseItemAck {
                agent_key,
                success: true,
            });
            self.add_broadcast(BroadcastMessage::OpenContainer {
                agent_key,
                guid,
                placement,
            });
        }

        Ok(())
    }
}
