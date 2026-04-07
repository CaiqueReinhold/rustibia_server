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
use tracing::{debug, error, info, warn};

use super::{session::SessionCommand, ActorHandle};
use crate::config::CONFIG;
use crate::entities::agent::{Agent, AgentKey};
use crate::entities::items::{ItemAttribute, ItemFlag, ItemGuid};
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
        message: String,
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
    UpdatePlayerCapacity {
        agent_key: AgentKey,
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
                self.add_broadcast(BroadcastMessage::MoveDenied {
                    agent_key: agent,
                    message: "Item not in reach".to_string(),
                });
                return Ok(());
            }
        }

        // --- Remove from source ---
        let source = match &from {
            ItemPlacement::Map(pos) => {
                self.map
                    .remove_item_from_tile(pos, item_guid, amount)
                    .map(|it| {
                        let change = if let Some((parent, _)) = &it.1 {
                            BroadcastMessage::UpdateContainer {
                                guid: parent.clone(),
                                placement: from.clone(),
                            }
                        } else {
                            BroadcastMessage::TileChanged {
                                position: pos.clone(),
                            }
                        };
                        (change, it)
                    })
            }
            ItemPlacement::Inventory(slot, agent_key) => self
                .map
                .get_player_mut(agent)
                .and_then(|player| player.inventory.remove(*slot, &item_guid, amount))
                .map(|it| {
                    let change = if let Some((parent, _)) = &it.1 {
                        BroadcastMessage::UpdateContainer {
                            guid: parent.clone(),
                            placement: from.clone(),
                        }
                    } else {
                        BroadcastMessage::UpdateInventorySlot {
                            agent_key: *agent_key,
                            slot: *slot,
                        }
                    };
                    (change, it)
                }),
        };

        let Some((source_change, (item, parent))) = source else {
            self.add_broadcast(BroadcastMessage::MoveDenied {
                agent_key: agent,
                message: "Can't move this".to_string(),
            });
            return Ok(());
        };

        // --- Add to target ---
        // parent container info as Option<(&guid, slot)> — used for displacing and rollback
        let parent_ref = parent.as_ref().map(|(guid, slot)| (guid, *slot));

        let target_change = match &to {
            ItemPlacement::Map(pos) => {
                let container = target_container.as_ref().map(|g| (g, 0usize));
                if self.map.place_item(pos, container, item.clone()).is_ok() {
                    Some(if let Some(guid) = target_container.as_ref() {
                        BroadcastMessage::UpdateContainer {
                            guid: guid.clone(),
                            placement: to.clone(),
                        }
                    } else {
                        BroadcastMessage::TileChanged {
                            position: pos.clone(),
                        }
                    })
                } else {
                    None
                }
            }
            ItemPlacement::Inventory(slot, _) => {
                let can_carry = self
                    .map
                    .get_player(agent)
                    .map(|player| player.can_carry(item.total_weight()))
                    .unwrap_or(false);
                if !can_carry {
                    self.add_broadcast(BroadcastMessage::MoveDenied {
                        agent_key: agent,
                        message: "Not enough capacity".to_string(),
                    });
                    return Ok(());
                }

                if let Some(target_container) = target_container.as_ref() {
                    // Moving into a container within the inventory slot
                    let result = self.map.get_player_mut(agent).map(|player| {
                        player
                            .inventory
                            .insert(*slot, Some((target_container, 0)), item.clone())
                    });
                    if let Some(result) = result {
                        if let Err(e) = result {
                            self.add_broadcast(BroadcastMessage::MoveDenied {
                                agent_key: agent,
                                message: e.to_string(),
                            });
                            return Ok(());
                        }
                        Some(BroadcastMessage::UpdateContainer {
                            guid: target_container.clone(),
                            placement: to.clone(),
                        })
                    } else {
                        None
                    }
                } else {
                    // Moving directly into an equipment slot
                    let current_item = self
                        .map
                        .get_player_mut(agent)
                        .and_then(|player| player.inventory.take_slot(slot));

                    // Displace any item currently in the slot back to the source
                    // (inventory-to-inventory swaps are rejected upstream, so from is always Map)
                    if let Some(current_item) = current_item {
                        if let ItemPlacement::Map(pos) = &from {
                            if self
                                .map
                                .place_item(pos, parent_ref, current_item.clone())
                                .is_err()
                            {
                                let fallback = self.map.agent_position(agent).cloned();
                                if let Some(fallback) = fallback {
                                    if self.map.place_item(&fallback, None, current_item).is_err() {
                                        error!("Failed to displace item on move. Agent: {:?}, Item: {:?}", agent, item);
                                    }
                                }
                            }
                        }
                    }

                    // if item is two handed, also remove shield and add it to first available container
                    if item.config.get_attributes().any(|attr| match attr {
                        ItemAttribute::Inventory(slot) => *slot == InventorySlot::BothHands,
                        _ => false,
                    }) {
                        // TODO
                    }

                    let player = self.map.get_player_mut(agent);
                    if let Some(player) = player {
                        let _ = player.inventory.insert(*slot, None, item.clone());
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
            // Restore item to its exact source position on failure
            match &from {
                ItemPlacement::Map(pos) => {
                    let _ = self.map.place_item(pos, parent_ref, item);
                }
                ItemPlacement::Inventory(slot, _) => {
                    if let Some(player) = self.map.get_player_mut(agent) {
                        let _ = player.inventory.insert(*slot, parent_ref, item);
                    }
                }
            }

            self.add_broadcast(BroadcastMessage::MoveDenied {
                agent_key: agent,
                message: "Can't move this".to_string(),
            });
            return Ok(());
        };

        let player = self.map.get_player_mut(agent);
        if let Some(player) = player {
            if player.capacity.current != player.inventory.carried_weight {
                player.capacity.current = player.inventory.carried_weight;
                self.add_broadcast(BroadcastMessage::UpdatePlayerCapacity { agent_key: agent });
            }
        }
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
            ItemPlacement::Inventory(slot, inv_agent_key) => self
                .map
                .get_player(*inv_agent_key)
                .and_then(|player| player.inventory.get(slot))
                .filter(|item| item.guid == guid),
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
