use anyhow::Result;
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
use crate::actors::item_action::{route_action, ItemActionError};
use crate::config::CONFIG;
use crate::entities::agent::{Agent, AgentKey, Facing};
use crate::entities::inventory::InventoryError;
use crate::entities::items::{ItemConfig, ItemFlag, ItemGuid, ItemId};
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
    ChangeDirection {
        agent: AgentKey,
        facing: Facing,
    },
    DespawnPlayer {
        agent_key: AgentKey,
        delay_ticks: Tick,
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
    AgentChangedDirection {
        agent_key: AgentKey,
        facing: Facing,
    },
    PlayerDespawned {
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
            WorldCommand::ChangeDirection { agent, facing } => {
                self.change_direction(agent, facing).await
            }
            WorldCommand::DespawnPlayer { agent_key, .. } => {
                self.map.remove_agent(agent_key);
                info!("Player {:?} despawned after disconnect", agent_key);
                self.add_broadcast(BroadcastMessage::PlayerDespawned { agent_key });
                Ok(())
            }
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
        if self.map.get_player(agent).is_none() {
            return Ok(());
        }

        let player_pos = self
            .map
            .agent_position(agent)
            .ok_or(anyhow::anyhow!("agent {:?} position not found", agent))?
            .clone();

        if let ItemPlacement::Map(pos) = &from {
            if !player_pos.is_adjacent(pos) {
                self.add_broadcast(BroadcastMessage::MoveDenied {
                    agent_key: agent,
                    message: "Item not in reach".to_string(),
                });
                return Ok(());
            }
        }

        // Validate source item: Unmove flag and stack amount.
        {
            let item = match &from {
                ItemPlacement::Map(pos) => self.map.get_item_by_id(pos, &item_guid),
                ItemPlacement::Inventory(slot, _) => self
                    .map
                    .get_player(agent)
                    .and_then(|p| p.inventory.get(slot))
                    .and_then(|it| it.find_by_guid(&item_guid)),
            };
            if let Some(item) = item {
                if item.config.has_flag(ItemFlag::Unmove) || item.amount < amount {
                    self.add_broadcast(BroadcastMessage::MoveDenied {
                        agent_key: agent,
                        message: "Can't move this".to_string(),
                    });
                    return Ok(());
                }
            }
        }

        // Validate target placement.
        match (&to, target_container.as_ref()) {
            (ItemPlacement::Map(pos), None) => {
                // Direct map drop: tile must accept items and be within viewport.
                if !self.map.can_drop_item(pos) || !player_pos.in_viewport(pos) {
                    self.add_broadcast(BroadcastMessage::MoveDenied {
                        agent_key: agent,
                        message: "Can't drop here".to_string(),
                    });
                    return Ok(());
                }
            }
            (ItemPlacement::Inventory(target_slot, _), None) => {
                // Direct equip: item must declare this as its valid slot.
                let item = match &from {
                    ItemPlacement::Map(pos) => self.map.get_item_by_id(pos, &item_guid),
                    ItemPlacement::Inventory(slot, _) => self
                        .map
                        .get_player(agent)
                        .and_then(|p| p.inventory.get(slot))
                        .and_then(|it| it.find_by_guid(&item_guid)),
                };
                let compatible = item
                    .and_then(|it| it.get_slot())
                    .map(|item_slot| {
                        item_slot == *target_slot
                            || (item_slot == InventorySlot::BothHands
                                && *target_slot == InventorySlot::LeftHand)
                    })
                    .unwrap_or(false);
                if !compatible {
                    self.add_broadcast(BroadcastMessage::MoveDenied {
                        agent_key: agent,
                        message: "Can't equip this here".to_string(),
                    });
                    return Ok(());
                }
            }
            (_, Some(container_guid)) => {
                // Placing into a container: item needs Take flag and can't go into itself.
                let item = match &from {
                    ItemPlacement::Map(pos) => self.map.get_item_by_id(pos, &item_guid),
                    ItemPlacement::Inventory(slot, _) => self
                        .map
                        .get_player(agent)
                        .and_then(|p| p.inventory.get(slot))
                        .and_then(|it| it.find_by_guid(&item_guid)),
                };
                let take_ok = item
                    .map(|it| it.config.has_flag(ItemFlag::Take))
                    .unwrap_or(false);
                if !take_ok || container_guid == &item_guid {
                    self.add_broadcast(BroadcastMessage::MoveDenied {
                        agent_key: agent,
                        message: "Can't move this".to_string(),
                    });
                    return Ok(());
                }
            }
        }

        // --- Remove from source ---
        let source = match &from {
            ItemPlacement::Map(pos) => {
                self.map
                    .remove_item_from_tile(pos, &item_guid, amount)
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

        let error_message = match &to {
            ItemPlacement::Map(pos) => {
                let container = target_container.as_ref().map(|g| (g, 0usize));
                match self.map.place_item(pos, container, item.clone()) {
                    Ok(..) => {
                        if let Some(guid) = target_container.as_ref() {
                            self.add_broadcast(BroadcastMessage::UpdateContainer {
                                guid: guid.clone(),
                                placement: to.clone(),
                            });
                        } else {
                            self.add_broadcast(BroadcastMessage::TileChanged {
                                position: pos.clone(),
                            });
                        }
                        None
                    }
                    Err(e) => Some(e.to_string()),
                }
            }
            ItemPlacement::Inventory(slot, _) => {
                let can_carry = self
                    .map
                    .get_player(agent)
                    .map(|player| player.can_carry(item.total_weight()))
                    .unwrap_or(false);

                if !can_carry {
                    Some("Not enough capacity".to_string())
                } else if let Some(target_container) = target_container.as_ref() {
                    // Moving into a container within the inventory slot
                    let result = self
                        .map
                        .get_player_mut(agent)
                        .map(|player| {
                            player.inventory.insert(
                                *slot,
                                Some((target_container, 0)),
                                item.clone(),
                            )
                        })
                        .unwrap();
                    match result {
                        Ok(..) => {
                            self.add_broadcast(BroadcastMessage::UpdateContainer {
                                guid: target_container.clone(),
                                placement: to.clone(),
                            });
                            None
                        }
                        Err(e) => Some(e.to_string()),
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

                    let mut error = None;
                    if item.get_slot().unwrap() == InventorySlot::BothHands {
                        info!("is two handed");
                        let player = self.map.get_player_mut(agent).unwrap();
                        if let Some(rh_item) = player.inventory.take_slot(&InventorySlot::RightHand)
                        {
                            info!("right hand item {:?}", rh_item);
                            if let Some(available_container) =
                                player.inventory.first_available_container().cloned()
                            {
                                if let Err(e) = player.inventory.insert(
                                    InventorySlot::Backpack,
                                    Some((&available_container, 0)),
                                    rh_item.clone(),
                                ) {
                                    let _ = player.inventory.insert(
                                        InventorySlot::RightHand,
                                        None,
                                        rh_item,
                                    );
                                    error = Some(e.to_string());
                                } else {
                                    self.add_broadcast(BroadcastMessage::UpdateInventorySlot {
                                        agent_key: agent,
                                        slot: InventorySlot::RightHand,
                                    });
                                    self.add_broadcast(BroadcastMessage::UpdateContainer {
                                        guid: available_container,
                                        placement: ItemPlacement::Inventory(
                                            InventorySlot::Backpack,
                                            agent,
                                        ),
                                    });
                                }
                            } else {
                                let _ = player.inventory.insert(
                                    InventorySlot::RightHand,
                                    None,
                                    rh_item,
                                );
                                error = Some(InventoryError::CannotEquip.to_string())
                            }
                        }
                    }

                    let left_is_two_handed = self
                        .map
                        .get_player_mut(agent)
                        .unwrap()
                        .inventory
                        .get(&InventorySlot::LeftHand)
                        .map(|it| it.get_slot().unwrap() == InventorySlot::BothHands)
                        .unwrap_or(false);
                    if *slot == InventorySlot::RightHand && left_is_two_handed {
                        let player = self.map.get_player_mut(agent).unwrap();
                        let lh_item = player
                            .inventory
                            .take_slot(&InventorySlot::LeftHand)
                            .unwrap();
                        if let Some(available_container) =
                            player.inventory.first_available_container().cloned()
                        {
                            if let Err(e) = player.inventory.insert(
                                InventorySlot::Backpack,
                                Some((&available_container, 0)),
                                lh_item.clone(),
                            ) {
                                let _ =
                                    player
                                        .inventory
                                        .insert(InventorySlot::LeftHand, None, lh_item);
                                error = Some(e.to_string());
                            }
                            self.add_broadcast(BroadcastMessage::UpdateInventorySlot {
                                agent_key: agent,
                                slot: InventorySlot::LeftHand,
                            });
                            self.add_broadcast(BroadcastMessage::UpdateContainer {
                                guid: available_container,
                                placement: ItemPlacement::Inventory(InventorySlot::Backpack, agent),
                            });
                        } else {
                            let _ = player
                                .inventory
                                .insert(InventorySlot::LeftHand, None, lh_item);
                            error = Some(InventoryError::CannotEquip.to_string())
                        }
                    }

                    if let Some(e) = error {
                        Some(e)
                    } else {
                        match self.map.get_player_mut(agent).unwrap().inventory.insert(
                            *slot,
                            None,
                            item.clone(),
                        ) {
                            Ok(..) => {
                                self.add_broadcast(BroadcastMessage::UpdateInventorySlot {
                                    agent_key: agent,
                                    slot: *slot,
                                });
                                None
                            }
                            Err(e) => Some(e.to_string()),
                        }
                    }
                }
            }
        };

        if let Some(error) = error_message {
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
                message: error,
            });
            return Ok(());
        }

        let player = self.map.get_player_mut(agent);
        if let Some(player) = player {
            if player.capacity.current != player.inventory.carried_weight {
                player.capacity.current = player.inventory.carried_weight;
                self.add_broadcast(BroadcastMessage::UpdatePlayerCapacity { agent_key: agent });
            }
        }
        self.add_broadcast(BroadcastMessage::MoveAck { agent_key: agent });
        self.add_broadcast(source_change);

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

        if !item.config.has_flag(ItemFlag::Usable) {
            self.add_broadcast(BroadcastMessage::UseItemAck {
                agent_key,
                success: false,
            });
            return Ok(());
        }

        let is_container = item.config.has_flag(ItemFlag::Container);
        let action = item.get_action();

        if is_container {
            self.add_broadcast(BroadcastMessage::UseItemAck {
                agent_key,
                success: true,
            });
            self.add_broadcast(BroadcastMessage::OpenContainer {
                agent_key,
                guid,
                placement,
            });
        } else if let Some(action) = action {
            let broadcasts = route_action(
                &action,
                &self.item_configs,
                &mut self.map,
                agent_key,
                &placement,
                &guid,
            );

            match broadcasts {
                Ok(broadcasts) => {
                    for msg in broadcasts {
                        self.add_broadcast(msg);
                    }
                    self.add_broadcast(BroadcastMessage::UseItemAck {
                        agent_key,
                        success: true,
                    });
                }
                Err(e) => {
                    if let ItemActionError::InvalidState = e {
                        warn!("{e}");
                    }
                    self.add_broadcast(BroadcastMessage::UseItemAck {
                        agent_key,
                        success: false,
                    });
                }
            };
        } else {
            self.add_broadcast(BroadcastMessage::UseItemAck {
                agent_key,
                success: false,
            });
        }

        Ok(())
    }

    async fn change_direction(&mut self, agent_key: AgentKey, facing: Facing) -> Result<()> {
        let current_facing = self.map.get_agent(agent_key).map(|agent| agent.facing);
        if let Some(current_facing) = current_facing {
            if facing != current_facing {
                let agent = self.map.get_agent_mut(agent_key).unwrap();
                agent.facing = facing;
                self.add_broadcast(BroadcastMessage::AgentChangedDirection { agent_key, facing });
            }
        }

        Ok(())
    }
}
