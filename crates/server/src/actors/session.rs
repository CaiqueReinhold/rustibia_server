use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use thiserror::Error;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::error;
use tracing::info;

use super::world::WorldCommand;
use crate::actors::SharedContext;
use crate::actors::chat::ChatActorHandle;
use crate::actors::connection::ConnectionActorHandle;
use crate::actors::persistence::PersistenceActorHandle;
use crate::actors::player_query::client_position_to_placement;
use crate::actors::player_query::get_agent_desc;
use crate::actors::player_query::get_player_desc;
use crate::actors::world::WorldActorHandle;
use crate::config::CONFIG;
use crate::entities::agent::Agent;
use crate::entities::agent::Facing;
use crate::entities::chat::ChannelId;
use crate::entities::chat::ChatMessageType;
use crate::entities::items::{ContainerId, ItemAttribute, ItemFlag, ItemGuid, ItemRef};
use crate::entities::player::InventorySlot;
use crate::entities::position::ItemPlacement;
use crate::game::Tick;
use crate::game::description::get_look_description;
use crate::game::events::BroadcastMessage;
use crate::game::game_config::GAME_CONFIG;
use crate::game::map_query::get_agents_in_expansion;
use crate::game::map_query::get_agents_in_viewport;
use crate::game::map_query::{
    find_item_in_reach, find_item_in_slot, find_parent_container, get_map_desc_on_viewport,
    get_map_expansion, get_tile, retrieve_item,
};
use crate::local_id::LocalIdMap;
use crate::messages::TextMessageType;
use crate::online_registry::RegistryGuard;
use crate::persistence::player::PlayerSnapshot;
use arc_swap::ArcSwap;

use crate::entities::{
    agent::AgentId,
    agent::AgentKey,
    items::ItemId,
    map::GameMap,
    position::{Direction, Position},
};
use crate::messages::{ClientMessage, ServerMessage};

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("Session failed to initialize")]
    FailedToInitialize,
    #[error("Message type unknown or out of order")]
    WrongMessageType,
    #[error("Player is not spawned")]
    NotSpawned,
    #[error("Invalid State")]
    InvalidState,
    #[error("Connection is closed")]
    ConnectionClosed,
    #[error("Player logged out")]
    Logout,
    #[error("World actor stopped")]
    WorldStopped,
}

#[derive(Clone, Debug)]
pub enum SessionCommand {
    PlayerMessage(ClientMessage),
    Broadcast(BroadcastMessage),
    ChatPrivate {
        author: AgentKey,
        message: String,
    },
    ChatChannel {
        author: AgentKey,
        channel: ChannelId,
        message: String,
    },
}

#[derive(Clone, Debug)]
pub struct SessionActorHandle {
    tx: mpsc::Sender<SessionCommand>,
    token: CancellationToken,
}

impl SessionActorHandle {
    pub fn close(&self) {
        self.token.cancel();
    }

    pub async fn receive_message(
        &self,
        msg: ClientMessage,
    ) -> Result<(), mpsc::error::SendError<SessionCommand>> {
        self.tx.send(SessionCommand::PlayerMessage(msg)).await?;
        Ok(())
    }

    pub fn receive_broadcast(
        &self,
        msg: BroadcastMessage,
    ) -> Result<(), mpsc::error::TrySendError<SessionCommand>> {
        self.tx.try_send(SessionCommand::Broadcast(msg))?;
        Ok(())
    }

    pub fn receive_chat_private(
        &self,
        author: AgentKey,
        message: String,
    ) -> Result<(), mpsc::error::TrySendError<SessionCommand>> {
        self.tx
            .try_send(SessionCommand::ChatPrivate { author, message })?;
        Ok(())
    }

    pub fn receive_chat_channel(
        &self,
        author: AgentKey,
        channel: ChannelId,
        message: String,
    ) -> Result<(), mpsc::error::TrySendError<SessionCommand>> {
        self.tx.try_send(SessionCommand::ChatChannel {
            author,
            channel,
            message,
        })?;
        Ok(())
    }

    /// A handle backed by a channel the caller owns. The receiver must be kept alive:
    /// dropping it turns every send into a `Closed` error, which the router reads as a
    /// dead session.
    #[cfg(test)]
    pub fn for_test() -> (Self, mpsc::Receiver<SessionCommand>) {
        let (tx, rx) = mpsc::channel(64);
        (
            Self {
                tx,
                token: CancellationToken::new(),
            },
            rx,
        )
    }
}

pub struct SessionActor {
    session_id: String,
    rx: mpsc::Receiver<SessionCommand>,
    token: CancellationToken,
    connection: ConnectionActorHandle,
    world: WorldActorHandle,
    player_key: AgentKey,
    shared_map: Arc<ArcSwap<GameMap>>,
    containers: LocalIdMap<ItemGuid>,
    agents: LocalIdMap<AgentKey>,
    player_pms: LocalIdMap<AgentKey>,
    persistence: PersistenceActorHandle,
    chat: ChatActorHandle,
    tick_rx: watch::Receiver<Tick>,
    next_chat_tick: Tick,
    queued_walk: Option<Direction>,
    logout_pending: bool,
}

/// What `SessionActor::for_test` hands back: the actor, plus the receiving end of every
/// channel it writes to and the tick sender. Every receiver has to stay *bound* for the
/// life of the test — dropping one turns the matching send into a silent no-op, which
/// would let an assertion about "nothing was sent" pass for the wrong reason.
#[cfg(test)]
type TestSession = (
    SessionActor,
    mpsc::Receiver<crate::actors::connection::ConnectionCommand>,
    mpsc::Receiver<(WorldCommand, Option<Tick>)>,
    watch::Sender<Tick>,
);

impl SessionActor {
    pub fn start(
        session_id: String,
        connection: ConnectionActorHandle,
        context: SharedContext,
        agent: Agent,
        registry_guard: RegistryGuard,
    ) -> SessionActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let token = CancellationToken::new();
        let self_handle = SessionActorHandle {
            tx,
            token: token.clone(),
        };

        let self_handle_clone = self_handle.clone();
        tokio::spawn(async move {
            let spawn_result = context
                .world
                .spawn_player(agent, self_handle_clone.clone())
                .await;
            match spawn_result {
                Ok((agent_key, message_router_guard)) => {
                    let _registry_guard = registry_guard;
                    let _router_guard = message_router_guard;
                    let actor = Self {
                        session_id,
                        rx,
                        token,
                        connection,
                        chat: context.chat.clone(),
                        world: context.world.clone(),
                        player_key: agent_key,
                        shared_map: context.shared_map.clone(),
                        containers: LocalIdMap::new(),
                        agents: LocalIdMap::new(),
                        player_pms: LocalIdMap::new(),
                        persistence: context.persistence.clone(),
                        tick_rx: context.tick_rx.clone(),
                        next_chat_tick: 0,
                        queued_walk: None,
                        logout_pending: false,
                    };
                    actor.run().await;
                }
                Err(e) => {
                    error!(session = session_id, "Failed to spawn player: {e}");
                    let _ = connection.close().await;
                }
            }
        });

        self_handle
    }

    async fn save_player(&self) {
        let map = self.shared_map.load();
        let Some(agent) = map.get_agent(self.player_key) else {
            return;
        };
        let Some(position) = map.agent_position(self.player_key) else {
            return;
        };
        let Some(snapshot) = agent.to_snapshot(position.clone()) else {
            return;
        };
        if let Err(e) = self.persistence.save_player(snapshot).await {
            error!(
                session = self.session_id,
                "Failed to queue player save: {e}"
            );
        }
    }

    async fn run(mut self) {
        info!(session = self.session_id, "Session actor started");

        let mut save_timer = tokio::time::interval(CONFIG.save_interval);
        save_timer.tick().await; // skip the immediate first tick

        loop {
            let result = select! { biased;
                _ = self.token.cancelled() => {
                    self.close_connection().await;
                    break;
                }
                cmd = self.rx.recv() =>
                    if let Some(cmd) = cmd {
                        self.route_command(cmd).await
                    } else {
                        Err(SessionError::ConnectionClosed.into())
                    },
                changed = self.tick_rx.changed() => {
                    if changed.is_err() {
                        // The world's tick sender was dropped. Without this the
                        // branch returns Ready forever and the session spins.
                        Err(SessionError::WorldStopped.into())
                    } else {
                        self.check_queues().await
                    }
                }
                _ = save_timer.tick() => {
                    self.save_player().await;
                    Ok(())
                }
            };
            if let Err(e) = result {
                if e.downcast_ref::<SessionError>()
                    .is_some_and(|e| matches!(e, SessionError::Logout))
                {
                    info!(session = self.session_id, "Player logged out cleanly");
                } else {
                    error!("Error on session command: {e}");
                }
                break;
            }
        }

        self.save_player().await;
        let _ = self.connection.close().await;

        let delay_ticks =
            (CONFIG.player_despawn_delay.as_millis() / CONFIG.tick_duration.as_millis()) as u64;
        let _ = self
            .world
            .send_delayed(
                WorldCommand::DespawnPlayer {
                    agent_key: self.player_key,
                },
                delay_ticks,
            )
            .await;
    }

    async fn close_connection(&self) {
        let _ = self.connection.close().await;
    }

    async fn route_command(&mut self, cmd: SessionCommand) -> Result<()> {
        info!(
            session = self.session_id,
            "Session received command: {:?}", cmd
        );
        match cmd {
            SessionCommand::PlayerMessage(msg) => self.handle_client_message(msg).await,
            SessionCommand::Broadcast(msg) => self.route_broadcast(msg).await,
            SessionCommand::ChatPrivate { author, message } => {
                self.receive_private_message(author, message).await
            }
            SessionCommand::ChatChannel {
                author,
                channel,
                message,
            } => self.receive_channel_message(author, channel, message).await,
        }
    }

    async fn pong(&self) -> Result<()> {
        self.connection.send_message(ServerMessage::Pong).await?;
        Ok(())
    }

    /// Resolves `agent_key` to a session-local chat id, sending `IntroducePlayer` the
    /// first time this session hears from them. `player_pms` is never pruned by the
    /// viewport, so a conversation survives the other party walking out of view.
    /// Returns `None` if the agent has left the map and can no longer be named.
    async fn introduce(&mut self, agent_key: AgentKey) -> Result<Option<AgentId>> {
        if let Some(local_id) = self.player_pms.get_local(&agent_key) {
            return Ok(Some(local_id));
        }

        // Scoped so the snapshot guard is released before the await below.
        let name = {
            let map = self.shared_map.load();
            match map.get_agent(agent_key) {
                Some(agent) => agent.name().to_owned(),
                None => return Ok(None),
            }
        };

        let local_id = self.player_pms.get_or_insert(agent_key);
        self.connection
            .send_message(ServerMessage::IntroducePlayer { local_id, name })
            .await?;
        Ok(Some(local_id))
    }

    async fn deny(&self, text: &str) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: text.to_owned(),
                message_type: TextMessageType::ActionDenied,
            })
            .await?;
        Ok(())
    }

    async fn send_chat(
        &mut self,
        author: AgentKey,
        message_type: ChatMessageType,
        channel: ChannelId,
        message: String,
    ) -> Result<()> {
        let Some(author) = self.introduce(author).await? else {
            return Ok(());
        };
        self.connection
            .send_message(ServerMessage::ChatMessage {
                author,
                message_type,
                channel,
                message,
            })
            .await?;
        Ok(())
    }

    async fn receive_private_message(&mut self, author: AgentKey, message: String) -> Result<()> {
        self.send_chat(author, ChatMessageType::Private, 0, message)
            .await
    }

    async fn receive_channel_message(
        &mut self,
        author: AgentKey,
        channel: ChannelId,
        message: String,
    ) -> Result<()> {
        self.send_chat(author, ChatMessageType::Channel, channel, message)
            .await
    }

    async fn handle_client_message(&mut self, command: ClientMessage) -> Result<()> {
        match command {
            ClientMessage::Ping => self.pong().await,
            ClientMessage::Login { .. } => Err(SessionError::WrongMessageType.into()),
            ClientMessage::MovePlayer { direction } => self.handle_move_player(direction).await,
            ClientMessage::GetPlayerPosition => self.handle_get_position().await,
            ClientMessage::MoveItem {
                from,
                item_id,
                amount,
                stack_index,
                to,
            } => {
                self.handle_move_item(from, item_id, amount, stack_index, to)
                    .await
            }
            ClientMessage::UseItem {
                position,
                item_id,
                stack_index,
            } => self.handle_use_item(position, item_id, stack_index).await,
            ClientMessage::CloseContainer { container_id } => {
                self.containers.remove_by_local(container_id);
                Ok(())
            }
            ClientMessage::OpenParentContainer { container_id } => {
                self.handle_open_parent_container(container_id).await
            }
            ClientMessage::ChangeDirection { direction } => {
                self.handle_change_direction(direction).await
            }
            ClientMessage::Logout => self.handle_logout().await,
            ClientMessage::UseItemWith {
                source,
                source_item_id,
                source_index,
                target,
                target_item_id,
                target_index,
            } => {
                self.handle_use_item_with(
                    source,
                    source_item_id,
                    source_index,
                    target,
                    target_item_id,
                    target_index,
                )
                .await
            }
            ClientMessage::Look { position } => self.handle_look(position).await,
            ClientMessage::Say {
                message,
                message_type,
                target,
            } => self.handle_say(message, message_type, target).await,
            ClientMessage::RequestChannels => self.handle_request_channels().await,
            ClientMessage::OpenChannel { channel } => self.handle_open_channel(channel).await,
            ClientMessage::CloseChannel { channel } => self.handle_close_channel(channel).await,
            ClientMessage::OpenPmChat { name } => self.handle_open_pm_chat(name).await,
        }
    }

    async fn handle_logout(&mut self) -> Result<()> {
        if !self.logout_pending {
            self.logout_pending = true;
            self.world
                .send(WorldCommand::RequestLogout {
                    agent_key: self.player_key,
                })
                .await;
        }
        Ok(())
    }

    /// Ticks remaining before this session's player may walk again, read from the
    /// published snapshot.
    ///
    /// `saturating_sub` is load-bearing: `next_walk_tick` trails the current tick
    /// by an unbounded amount once a player has stood still, so a plain subtraction
    /// underflows the `u64` and every idle walk looks like a ~2^64-tick wait.
    ///
    /// An agent missing from the snapshot reads as 0. The session must not start
    /// refusing walks on its own reading of a snapshot that may not yet contain a
    /// just-spawned player.
    fn walk_cooldown_remaining(&self) -> Tick {
        let map = self.shared_map.load();
        map.get_agent(self.player_key)
            .map(|agent| agent.next_walk_tick.saturating_sub(*self.tick_rx.borrow()))
            .unwrap_or(0)
    }

    /// Per-tick hook for session-local state that advances with the world clock.
    /// Today only the walk queue; further per-tick cooldowns belong here rather
    /// than in their own `select!` branch.
    async fn check_queues(&mut self) -> Result<()> {
        self.check_walk_queue().await
    }

    /// Recomputes the remaining cooldown from the snapshot every tick rather than
    /// storing a deadline at admission, so it self-corrects if the cooldown moves.
    async fn check_walk_queue(&mut self) -> Result<()> {
        let Some(direction) = self.queued_walk else {
            return Ok(());
        };
        if self.walk_cooldown_remaining() > 0 {
            return Ok(());
        }
        self.queued_walk = None;
        self.send_walk(direction).await
    }

    async fn send_walk(&self, direction: Direction) -> Result<()> {
        self.world
            .send(WorldCommand::Walk {
                direction,
                actor: self.player_key,
            })
            .await;
        Ok(())
    }

    /// A walk that arrives while the cooldown is nearly up is held rather than
    /// forwarded into a refusal. The client's step animation and the server's
    /// cooldown are the same length, but the client starts its clock when it sends
    /// and the server when it processes, so a walk clears by exactly zero ticks and
    /// any downward latency jitter refuses it.
    ///
    /// A walk that is early by more than the window is still forwarded: the
    /// snapshot read here can be a tick stale, so the world stays the single
    /// authority on refusal. That case is self-correcting — the client's next
    /// key-repeat retry arrives closer and lands inside the window.
    async fn handle_move_player(&mut self, direction: Direction) -> Result<()> {
        let remaining = self.walk_cooldown_remaining();
        if remaining > 0 && remaining <= GAME_CONFIG.movement.walk_queue_ticks {
            // Newest wins: a direction change supersedes what was queued.
            self.queued_walk = Some(direction);
            return Ok(());
        }
        // A fresh walk supersedes a queued one, so a stale direction cannot fire
        // behind it.
        self.queued_walk = None;
        self.send_walk(direction).await
    }

    async fn handle_get_position(&self) -> Result<()> {
        let map = self.shared_map.load();
        if let Some(position) = map.agent_position(self.player_key) {
            self.connection
                .send_message(ServerMessage::PlayerPosition {
                    position: position.clone(),
                })
                .await?;
        } else {
            return Err(SessionError::WrongMessageType.into());
        }
        Ok(())
    }

    async fn handle_move_item(
        &self,
        from: Position,
        item_id: ItemId,
        amount: u8,
        stack_index: u8,
        to: Position,
    ) -> Result<()> {
        let map = self.shared_map.load();
        let player_key = self.player_key;

        // Resolve source: Position → (item_guid, ItemPlacement).
        // Uses the session-local container map to translate container coords.
        let Some((item, source_placement)) = retrieve_item(
            &map,
            &from,
            item_id,
            stack_index,
            &self.containers,
            player_key,
        ) else {
            return Ok(());
        };
        let item_guid = item.guid.clone();

        // Resolve target: Position → (ItemPlacement, Option<container_guid>).
        let (target_placement, target_container) = if to.is_container_coord() {
            let container_id = to.y as ContainerId;
            let Some(guid) = self.containers.get_global(container_id) else {
                return Ok(());
            };
            let Some((container, placement)) = find_item_in_reach(&map, guid, player_key) else {
                return Ok(());
            };
            // If the target slot holds a container, redirect into it.
            let slot = to.z as usize;
            let effective_guid = container
                .content
                .as_ref()
                .and_then(|c| c.get(slot))
                .filter(|it| it.config.has_flag(ItemFlag::Container))
                .map(|it| it.guid.clone())
                .unwrap_or_else(|| container.guid.clone());
            (placement, Some(effective_guid))
        } else if to.is_inventory_coord() {
            let Some(target_slot) = InventorySlot::from_id(to.y) else {
                return Ok(());
            };
            (ItemPlacement::Inventory(target_slot, player_key), None)
        } else {
            (ItemPlacement::Map(to), None)
        };

        self.world
            .send(WorldCommand::MoveItem {
                agent: player_key,
                source: ItemRef {
                    guid: item_guid,
                    placement: source_placement,
                },
                amount,
                to: target_placement,
                target_container,
            })
            .await;

        Ok(())
    }

    async fn handle_use_item(
        &self,
        position: Position,
        item_id: ItemId,
        stack_index: u8,
    ) -> Result<()> {
        let map = self.shared_map.load();

        let Some((item, placement)) = retrieve_item(
            &map,
            &position,
            item_id,
            stack_index,
            &self.containers,
            self.player_key,
        ) else {
            return Ok(());
        };

        self.world
            .send(WorldCommand::UseItem {
                agent: self.player_key,
                item: ItemRef {
                    guid: item.guid.clone(),
                    placement,
                },
            })
            .await;

        Ok(())
    }

    async fn handle_use_item_with(
        &self,
        source: Position,
        source_item_id: ItemId,
        source_index: u8,
        target: Position,
        target_item_id: ItemId,
        target_index: u8,
    ) -> Result<()> {
        let map = self.shared_map.load();

        let Some((source_item, source_placement)) = retrieve_item(
            &map,
            &source,
            source_item_id,
            source_index,
            &self.containers,
            self.player_key,
        ) else {
            return Ok(());
        };

        let Some((target_item, target_placement)) = retrieve_item(
            &map,
            &target,
            target_item_id,
            target_index,
            &self.containers,
            self.player_key,
        ) else {
            return Ok(());
        };

        self.world
            .send(WorldCommand::UseItemWith {
                agent: self.player_key,
                source: ItemRef {
                    guid: source_item.guid.clone(),
                    placement: source_placement,
                },
                target: ItemRef {
                    guid: target_item.guid.clone(),
                    placement: target_placement,
                },
            })
            .await;

        Ok(())
    }

    async fn handle_open_parent_container(&mut self, container_id: ContainerId) -> Result<()> {
        let container_guid = self.containers.get_global(container_id);
        if let Some(guid) = container_guid {
            let map = self.shared_map.load();
            let container = find_parent_container(&map, guid, self.player_key);
            if let Some((parent_guid, placement)) = container {
                return self
                    .open_container(ItemRef {
                        guid: parent_guid.clone(),
                        placement,
                    })
                    .await;
            }
        }

        Ok(())
    }

    async fn handle_change_direction(&self, facing: Facing) -> Result<()> {
        self.world
            .send(WorldCommand::ChangeDirection {
                agent: self.player_key,
                facing,
            })
            .await;
        Ok(())
    }

    async fn handle_look(&self, position: Position) -> Result<()> {
        let map = self.shared_map.load();
        let player_pos = map
            .agent_position(self.player_key)
            .ok_or(SessionError::InvalidState)?;
        let Some((placement, guid)) =
            client_position_to_placement(position, &map, &self.containers, self.player_key)
        else {
            return Ok(());
        };
        let desc = get_look_description(&map, &placement, guid, player_pos);
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: desc,
                message_type: TextMessageType::Look,
            })
            .await?;
        Ok(())
    }

    async fn handle_say(
        &mut self,
        message: String,
        message_type: ChatMessageType,
        target: u16,
    ) -> Result<()> {
        // Characters, not bytes, so the client's input-field cap measures the same thing
        // and no message the player could compose is refused for length.
        if message.chars().count() > GAME_CONFIG.chat.max_message_length {
            return self.deny("Your message is too long.").await;
        }

        let now = *self.tick_rx.borrow();
        if now < self.next_chat_tick {
            return self.deny("You are sending messages too fast.").await;
        }
        self.next_chat_tick = now + GAME_CONFIG.chat.message_cooldown_ticks;

        match message_type {
            ChatMessageType::Local => {
                self.world
                    .send(WorldCommand::Say {
                        agent_key: self.player_key,
                        message,
                    })
                    .await;
            }
            ChatMessageType::Private => {
                // `target` is a `player_pms` id: the client got it from an
                // `IntroducePlayer`, either by opening the chat or by receiving a message.
                if let Some(recipient) = self.player_pms.get_global(target).copied() {
                    self.chat
                        .message_player(self.player_key, recipient, message)
                        .await;
                }
            }
            ChatMessageType::Channel => {
                self.chat
                    .message_channel(self.player_key, target, message)
                    .await;
            }
        }
        Ok(())
    }

    /// Resolves a name to a chat id so the client can start a conversation. The map
    /// snapshot is the only name index there is — `OnlineRegistry` holds character ids
    /// and nothing else.
    async fn handle_open_pm_chat(&mut self, name: String) -> Result<()> {
        let target = {
            let map = self.shared_map.load();
            map.iter_agents()
                .find(|(_, agent)| !agent.is_creature() && agent.name().eq_ignore_ascii_case(&name))
                .map(|(key, _)| key)
        };

        let Some(target) = target else {
            return self.deny("A player with this name is not online.").await;
        };

        self.introduce(target).await?;
        Ok(())
    }

    async fn handle_request_channels(&self) -> Result<()> {
        let channels = self
            .chat
            .get_available_channels()
            .map(|(id, name)| (id, name.to_owned()))
            .collect();
        self.connection
            .send_message(ServerMessage::ChannelList { channels })
            .await?;
        Ok(())
    }

    async fn handle_open_channel(&self, channel: ChannelId) -> Result<()> {
        self.chat.join_channel(self.player_key, channel).await;
        Ok(())
    }

    async fn handle_close_channel(&self, channel: ChannelId) -> Result<()> {
        self.chat.leave_channel(self.player_key, channel).await;
        Ok(())
    }

    async fn route_broadcast(&mut self, msg: BroadcastMessage) -> Result<()> {
        info!(
            session = self.session_id,
            "Session received broadcast: {:?}", msg
        );
        match msg {
            BroadcastMessage::AgentMoved {
                agent_key,
                direction,
                to_position,
                ..
            } => self.agent_moved(agent_key, direction, to_position).await,
            BroadcastMessage::PlayerSpawned {
                agent_key,
                position,
            } => self.player_spawned(agent_key, position).await,
            BroadcastMessage::MoveItemDenied { message, .. } => {
                self.move_item_denied(message).await
            }
            BroadcastMessage::TileChanged { position } => self.tile_changed(position).await,
            BroadcastMessage::UseItemDenied { message, .. } => self.use_item_denied(message).await,
            BroadcastMessage::OpenContainer { item, .. } => self.open_container(item).await,
            BroadcastMessage::UpdateContainer { item } => self.update_container(item).await,
            BroadcastMessage::AgentWalkDenied { .. } => self.walk_denied().await,
            BroadcastMessage::UpdateInventorySlot { agent_key, slot } => {
                self.update_inventory_slot(agent_key, slot).await
            }
            BroadcastMessage::UpdatePlayerCapacity { agent_key } => {
                self.update_player_capacity(agent_key).await
            }
            BroadcastMessage::AgentChangedDirection {
                agent_key, facing, ..
            } => self.actor_direction_changed(agent_key, facing).await,
            BroadcastMessage::PlayerDespawned {
                agent_key,
                snapshot,
                ..
            } => self.agent_despawned(agent_key, snapshot).await,
            BroadcastMessage::AgentTeleport {
                agent_key,
                to_position,
                ..
            } => self.agent_teleported(agent_key, to_position).await,
            BroadcastMessage::LogoutDenied { .. } => self.logout_denied().await,
            BroadcastMessage::AgentSaid { agent_key, message } => {
                self.agent_said(agent_key, message).await
            }
        }
    }

    async fn player_spawned(&mut self, agent_key: AgentKey, position: Position) -> Result<()> {
        let map = self.shared_map.load();
        if self.player_key == agent_key {
            let self_id = self.agents.get_or_insert(self.player_key);

            self.send_map_description(&position, &map).await?;
            self.send_agents_description(&position, &map).await?;

            let player_desc = get_player_desc(&map, self.player_key, self_id);
            if let Some(pdesc_msg) = player_desc {
                self.connection.send_message(pdesc_msg).await?;
            } else {
                return Err(SessionError::FailedToInitialize.into());
            }
            Ok(())
        } else {
            let Some(agent) = map.get_agent(agent_key) else {
                return Ok(());
            };
            let agent_id = self.agents.get_or_insert(agent_key);

            self.connection
                .send_message(get_agent_desc(agent, agent_id, position))
                .await?;

            Ok(())
        }
    }

    async fn send_agents_description(
        &mut self,
        position: &Position,
        map: &GameMap,
    ) -> Result<HashSet<AgentKey>> {
        let mut visible = HashSet::new();
        for (key, agent, pos) in get_agents_in_viewport(map, position) {
            if key == self.player_key {
                continue;
            }
            let agent_id = self.agents.get_or_insert(key);
            self.connection
                .send_message(get_agent_desc(agent, agent_id, pos))
                .await?;
            visible.insert(key);
        }
        Ok(visible)
    }

    async fn send_map_description(&self, position: &Position, map: &GameMap) -> Result<()> {
        let map_desc_floors = get_map_desc_on_viewport(map, position);
        for (floor, tiles) in map_desc_floors {
            self.connection
                .send_message(ServerMessage::DescribeMap {
                    tiles,
                    center: position.clone(),
                    floor,
                })
                .await?;
        }
        Ok(())
    }

    async fn drop_unreachble_containers(&mut self) -> Result<()> {
        let map = self.shared_map.load();
        let mut remove: Vec<ContainerId> = Vec::new();
        for guid in self.containers.iter_global() {
            if find_item_in_reach(&map, guid, self.player_key).is_none() {
                remove.push(self.containers.get_local(guid).unwrap());
            }
        }
        for id in remove {
            self.containers.remove_by_local(id);
            self.connection
                .send_message(ServerMessage::ContainerClosed { container_id: id })
                .await?;
        }
        Ok(())
    }

    async fn agent_moved(
        &mut self,
        agent_key: AgentKey,
        direction: Direction,
        to_position: Position,
    ) -> Result<()> {
        let map = self.shared_map.load();
        if self.player_key == agent_key {
            self.drop_unreachble_containers().await?;

            for (key, agent, pos) in get_agents_in_expansion(&map, &to_position, &direction) {
                let agent_id = self.agents.get_or_insert(key);
                self.connection
                    .send_message(get_agent_desc(agent, agent_id, pos))
                    .await?;
            }

            let tiles = {
                let from_pos = to_position.clone() - direction;
                get_map_expansion(&map, &from_pos, &direction)
            };
            self.connection
                .send_message(ServerMessage::PlayerWalkAck {
                    position: to_position.clone(),
                    tiles,
                })
                .await?;

            Ok(())
        } else {
            let Some(my_pos) = map.agent_position(self.player_key) else {
                return Ok(());
            };

            if my_pos.in_viewport(&to_position) {
                if let Some(agent_id) = self.agents.get_local(&agent_key) {
                    let from = to_position.clone() - direction;
                    self.connection
                        .send_message(ServerMessage::MoveAgent {
                            agent_id,
                            direction,
                            from,
                        })
                        .await?;
                } else {
                    let Some(agent) = map.get_agent(agent_key) else {
                        return Ok(());
                    };
                    let agent_id = self.agents.get_or_insert(agent_key);

                    self.connection
                        .send_message(get_agent_desc(agent, agent_id, to_position))
                        .await?;
                }
            } else if let Some(agent_id) = self.agents.get_local(&agent_key) {
                self.agents.remove_by_local(agent_id);
                self.connection
                    .send_message(ServerMessage::RemoveAgent { agent_id })
                    .await?;
            }

            Ok(())
        }
    }

    async fn walk_denied(&self) -> Result<()> {
        self.connection
            .send_message(ServerMessage::PlayerWalkDenied)
            .await?;

        Ok(())
    }

    async fn move_item_denied(&self, message: String) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: message,
                message_type: TextMessageType::ActionDenied,
            })
            .await?;

        Ok(())
    }

    async fn tile_changed(&mut self, position: Position) -> Result<()> {
        self.drop_unreachble_containers().await?;
        let map = self.shared_map.load();
        let player_pos = map
            .agent_position(self.player_key)
            .ok_or(SessionError::NotSpawned)?;

        if player_pos.in_viewport(&position) {
            let tile = get_tile(&map, &position);
            self.connection
                .send_message(ServerMessage::TileChanged {
                    position,
                    items: tile,
                })
                .await?;
        }

        Ok(())
    }

    async fn use_item_denied(&self, message: String) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: message,
                message_type: TextMessageType::ActionDenied,
            })
            .await?;

        Ok(())
    }

    async fn open_container(&mut self, item_ref: ItemRef) -> Result<()> {
        let map = self.shared_map.load();
        let item = match &item_ref.placement {
            ItemPlacement::Map(position) => {
                let item = map.get_item_by_id(position, &item_ref.guid);
                let Some(item) = item else {
                    return Err(SessionError::InvalidState.into());
                };
                item
            }
            ItemPlacement::Inventory(slot, agent_key) => {
                let Some(agent) = map.get_agent(*agent_key) else {
                    return Err(SessionError::InvalidState.into());
                };
                let Some(item) = find_item_in_slot(agent, *slot, &item_ref.guid) else {
                    return Err(SessionError::InvalidState.into());
                };
                item
            }
        };

        let Some(capacity) = item.config.get_attributes().find_map(|attr| match attr {
            ItemAttribute::Capacity(c) => Some(c),
            _ => None,
        }) else {
            return Err(SessionError::InvalidState.into());
        };
        let capacity = *capacity;
        let Some(ref content) = item.content else {
            return Err(SessionError::InvalidState.into());
        };

        let title = item.get_name().to_owned();
        let items = content
            .iter()
            .map(|i| Some((i.item_id, i.amount)))
            .collect::<Vec<Option<(ItemId, u8)>>>()
            .into_boxed_slice();
        let container_id = self.containers.get_or_insert(item_ref.guid.clone());
        let has_parent = find_parent_container(&map, &item_ref.guid, self.player_key).is_some();

        self.connection
            .send_message(ServerMessage::OpenContainer {
                container_id,
                capacity,
                has_parent,
                title,
                items,
            })
            .await?;

        Ok(())
    }

    async fn update_container(&mut self, item_ref: ItemRef) -> Result<()> {
        if let Some(local_id) = self.containers.get_local(&item_ref.guid) {
            let map = self.shared_map.load();
            let item = match &item_ref.placement {
                ItemPlacement::Map(position) => {
                    let item = map.get_item_by_id(position, &item_ref.guid);
                    let Some(item) = item else {
                        return Err(SessionError::InvalidState.into());
                    };
                    item
                }
                ItemPlacement::Inventory(slot, agent_key) => {
                    let Some(agent) = map.get_agent(*agent_key) else {
                        return Err(SessionError::InvalidState.into());
                    };
                    let Some(item) = find_item_in_slot(agent, *slot, &item_ref.guid) else {
                        return Err(SessionError::InvalidState.into());
                    };
                    item
                }
            };

            let Some(content) = &item.content else {
                return Err(SessionError::InvalidState.into());
            };

            let items = content
                .iter()
                .map(|i| Some((i.item_id, i.amount)))
                .collect::<Vec<Option<(ItemId, u8)>>>()
                .into_boxed_slice();

            self.connection
                .send_message(ServerMessage::UpdateContainer {
                    container_id: local_id,
                    items,
                })
                .await?;
        }

        Ok(())
    }

    async fn update_inventory_slot(
        &mut self,
        agent_key: AgentKey,
        slot: InventorySlot,
    ) -> Result<()> {
        self.drop_unreachble_containers().await?;
        let map = self.shared_map.load();
        let Some(agent) = map.get_agent(agent_key) else {
            return Ok(());
        };
        let Some(player) = agent.get_player() else {
            return Ok(());
        };
        let item_id = player.inventory.get(&slot).map(|it| it.item_id);
        self.connection
            .send_message(ServerMessage::IventorySlotUpdated { slot, item_id })
            .await?;

        Ok(())
    }

    async fn update_player_capacity(&self, agent_key: AgentKey) -> Result<()> {
        let map = self.shared_map.load();
        if let Some(cap) = map.get_player(agent_key).map(|player| &player.capacity) {
            self.connection
                .send_message(ServerMessage::PlayerCapacityUpdated {
                    cap: cap.available(),
                })
                .await?;
        } else {
            return Err(SessionError::InvalidState.into());
        }

        Ok(())
    }

    async fn actor_direction_changed(&self, agent_key: AgentKey, facing: Facing) -> Result<()> {
        if let Some(agent_id) = self.agents.get_local(&agent_key) {
            self.connection
                .send_message(ServerMessage::AgentChangedDirection { agent_id, facing })
                .await?;
        }
        Ok(())
    }

    async fn logout_denied(&self) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: "You may not logout during an action.".to_string(),
                message_type: TextMessageType::ActionDenied,
            })
            .await?;
        Ok(())
    }

    async fn agent_despawned(
        &mut self,
        agent_key: AgentKey,
        snapshot: Option<Arc<PlayerSnapshot>>,
    ) -> Result<()> {
        if self.player_key == agent_key {
            if let Some(snapshot) = snapshot {
                if let Err(e) = self
                    .persistence
                    .save_player(snapshot.as_ref().clone())
                    .await
                {
                    error!(
                        session = self.session_id,
                        "Failed to save player on logout: {e}"
                    );
                }
                return Err(SessionError::Logout.into());
            }
            return Ok(());
        }

        if let Some(agent_id) = self.agents.get_local(&agent_key) {
            self.agents.remove_by_local(agent_id);
            self.connection
                .send_message(ServerMessage::RemoveAgent { agent_id })
                .await?;
        }
        Ok(())
    }

    async fn agent_teleported(&mut self, agent_key: AgentKey, to_position: Position) -> Result<()> {
        let map = self.shared_map.load();
        if agent_key == self.player_key {
            self.send_map_description(&to_position, &map).await?;
            let visible = self.send_agents_description(&to_position, &map).await?;

            let self_id = self
                .agents
                .get_local(&self.player_key)
                .ok_or(SessionError::InvalidState)?;
            self.connection
                .send_message(ServerMessage::TeleportAgent {
                    agent_id: self_id,
                    position: to_position.clone(),
                })
                .await?;

            self.remove_agents_not_in_reach(visible).await?;

            Ok(())
        } else {
            let Some(my_pos) = map.agent_position(self.player_key) else {
                return Ok(());
            };

            if my_pos.in_viewport(&to_position) {
                if let Some(agent_id) = self.agents.get_local(&agent_key) {
                    self.connection
                        .send_message(ServerMessage::TeleportAgent {
                            agent_id,
                            position: to_position,
                        })
                        .await?;
                } else {
                    let Some(agent) = map.get_agent(agent_key) else {
                        return Ok(());
                    };
                    let agent_id = self.agents.get_or_insert(agent_key);
                    self.connection
                        .send_message(get_agent_desc(agent, agent_id, to_position))
                        .await?;
                }
            } else if let Some(agent_id) = self.agents.get_local(&agent_key) {
                self.agents.remove_by_local(agent_id);
                self.connection
                    .send_message(ServerMessage::RemoveAgent { agent_id })
                    .await?;
            }

            Ok(())
        }
    }

    async fn remove_agents_not_in_reach(&mut self, visible: HashSet<AgentKey>) -> Result<()> {
        for agent_id in self
            .agents
            .iter_global()
            .filter(|key| **key != self.player_key && !visible.contains(key))
            .filter_map(|key| self.agents.get_local(key))
            .collect::<Vec<_>>()
        {
            self.connection
                .send_message(ServerMessage::RemoveAgent { agent_id })
                .await?;
            self.agents.remove_by_local(agent_id);
        }

        Ok(())
    }

    async fn agent_said(&mut self, agent_key: AgentKey, message: String) -> Result<()> {
        self.send_chat(agent_key, ChatMessageType::Local, 0, message)
            .await
    }

    #[cfg(test)]
    fn for_test(player_key: AgentKey, map: GameMap) -> TestSession {
        use crate::actors::chat::ChatActorHandle;
        use crate::actors::world::WorldActorHandle;

        let (_tx, rx) = mpsc::channel(64);
        let (connection, connection_rx) = ConnectionActorHandle::for_test();
        let (world, world_rx) = WorldActorHandle::for_test();
        let (chat, _chat_rx) = ChatActorHandle::for_test();
        let (persistence, _persistence_rx) = PersistenceActorHandle::for_test(16);
        let (tick_tx, tick_rx) = watch::channel(0);

        (
            Self {
                session_id: "test".to_owned(),
                rx,
                token: CancellationToken::new(),
                connection,
                world,
                chat,
                player_key,
                shared_map: Arc::new(ArcSwap::from_pointee(map)),
                containers: LocalIdMap::new(),
                agents: LocalIdMap::new(),
                player_pms: LocalIdMap::new(),
                persistence,
                tick_rx,
                next_chat_tick: 0,
                queued_walk: None,
                logout_pending: false,
            },
            connection_rx,
            world_rx,
            tick_tx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::connection::ConnectionCommand;
    use crate::entities::map::MapTile;
    use crate::entities::position::Position;
    use crate::persistence::test_fixtures::a_test_snapshot;

    fn seat_player(map: &mut GameMap, at: &Position, id: u32) -> AgentKey {
        map.insert_tile(at.clone(), MapTile::new());
        map.insert_agent(Agent::from_player(a_test_snapshot(id, 1)), at)
            .unwrap()
    }

    #[tokio::test]
    async fn an_author_is_introduced_exactly_once() {
        let mut map = GameMap::new();
        let author_a = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let author_b = seat_player(&mut map, &Position::new(101, 100, 7), 2);
        let (mut session, mut connection_rx, _world_rx, _tick_tx) =
            SessionActor::for_test(author_a, map);

        // Hearing from B in between is what makes the third call meaningful: a session
        // that forgot A would hand out a different id for A the second time round.
        let first_a = session.introduce(author_a).await.unwrap();
        let b = session.introduce(author_b).await.unwrap();
        let second_a = session.introduce(author_a).await.unwrap();

        assert_ne!(
            first_a, b,
            "two authors heard from in the same session must hold distinct local ids"
        );
        assert_eq!(
            first_a, second_a,
            "hearing from another author in between must not renumber the first"
        );
        assert!(
            matches!(
                connection_rx.try_recv(),
                Ok(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::IntroducePlayer { .. }
                ))
            ),
            "the first author is named over the wire"
        );
        assert!(
            matches!(
                connection_rx.try_recv(),
                Ok(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::IntroducePlayer { .. }
                ))
            ),
            "the second author is named over the wire"
        );
        assert!(
            connection_rx.try_recv().is_err(),
            "re-introducing a known author must not go back over the wire"
        );
    }

    #[tokio::test]
    async fn an_author_no_longer_on_the_map_is_not_introduced() {
        let (mut session, _connection_rx, _world_rx, _tick_tx) =
            SessionActor::for_test(AgentKey::default(), GameMap::new());

        assert_eq!(session.introduce(AgentKey::default()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_over_length_message_is_denied() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, mut connection_rx, _world_rx, _tick_tx) =
            SessionActor::for_test(key, map);

        let too_long = "x".repeat(GAME_CONFIG.chat.max_message_length + 1);
        session
            .handle_say(too_long, ChatMessageType::Local, 0)
            .await
            .unwrap();

        assert!(
            matches!(
                connection_rx.try_recv(),
                Ok(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::TextMessage {
                        message_type: TextMessageType::ActionDenied,
                        ..
                    }
                ))
            ),
            "an over-length message must be refused, not truncated"
        );
    }

    /// The limit counts characters, not bytes, so that the client's input-field cap
    /// measures the same thing. `"é"` is two bytes, so a message of exactly the limit is
    /// over the limit by the old byte rule and within it by the current one — an
    /// all-ASCII test cannot tell the two apart.
    #[tokio::test]
    async fn the_length_limit_counts_characters_not_bytes() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, mut connection_rx, mut world_rx, _tick_tx) =
            SessionActor::for_test(key, map);

        let at_limit = "é".repeat(GAME_CONFIG.chat.max_message_length);
        assert!(
            at_limit.len() > GAME_CONFIG.chat.max_message_length,
            "the fixture must exceed the limit in bytes, or it proves nothing"
        );

        session
            .handle_say(at_limit, ChatMessageType::Local, 0)
            .await
            .unwrap();

        assert!(
            connection_rx.try_recv().is_err(),
            "a message at the character limit must not be denied"
        );
        assert!(
            matches!(world_rx.try_recv(), Ok((WorldCommand::Say { .. }, _))),
            "a message at the character limit must reach the world"
        );
    }

    /// Enforcement is pinned on the *world* receiver, not the connection. Local speech is
    /// forwarded to `WorldCommand::Say` and never echoed back down the connection, so a
    /// denial arriving on `connection_rx` says the guard fired but not that the message
    /// was actually withheld — only the world receiver shows that. Both are asserted: one
    /// for enforcement, one for the player-facing feedback.
    #[tokio::test]
    async fn a_second_message_inside_the_cooldown_is_dropped() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, mut connection_rx, mut world_rx, tick_tx) =
            SessionActor::for_test(key, map);

        session
            .handle_say("one".to_owned(), ChatMessageType::Local, 0)
            .await
            .unwrap();
        assert!(
            matches!(world_rx.try_recv(), Ok((WorldCommand::Say { .. }, _))),
            "the first message must reach the world"
        );

        session
            .handle_say("two".to_owned(), ChatMessageType::Local, 0)
            .await
            .unwrap();
        assert!(
            world_rx.try_recv().is_err(),
            "a second message inside the cooldown must not reach the world"
        );

        assert!(
            matches!(
                connection_rx.try_recv(),
                Ok(ConnectionCommand::SendPlayerMessage(
                    ServerMessage::TextMessage {
                        message_type: TextMessageType::ActionDenied,
                        ..
                    }
                ))
            ),
            "the player must be told why the message did not go through"
        );
        assert!(
            session.next_chat_tick > 0,
            "the cooldown must have been armed"
        );

        // Once the cooldown elapses the same message does get through, so what is being
        // pinned is a delay and not a permanent mute.
        tick_tx
            .send(GAME_CONFIG.chat.message_cooldown_ticks)
            .unwrap();
        session
            .handle_say("three".to_owned(), ChatMessageType::Local, 0)
            .await
            .unwrap();
        assert!(
            matches!(world_rx.try_recv(), Ok((WorldCommand::Say { .. }, _))),
            "a message sent after the cooldown elapses must reach the world"
        );
    }

    #[tokio::test]
    async fn opening_a_pm_chat_with_an_offline_name_is_denied() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, mut connection_rx, _world_rx, _tick_tx) =
            SessionActor::for_test(key, map);

        session
            .handle_open_pm_chat("Nobody".to_owned())
            .await
            .unwrap();

        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::TextMessage { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn opening_a_pm_chat_introduces_the_target() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, mut connection_rx, _world_rx, _tick_tx) =
            SessionActor::for_test(key, map);

        // `a_test_snapshot` names the character "Rizael"; matching is case-insensitive.
        session
            .handle_open_pm_chat("rizael".to_owned())
            .await
            .unwrap();

        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::IntroducePlayer { .. }
            ))
        ));
    }

    /// Puts the seated player on cooldown as though it had just walked.
    fn arm_cooldown(session: &SessionActor, until: Tick) {
        let mut map = (**session.shared_map.load()).clone();
        map.get_agent_mut(session.player_key)
            .unwrap()
            .next_walk_tick = until;
        session.shared_map.store(Arc::new(map));
    }

    #[tokio::test]
    async fn a_walk_with_no_cooldown_is_forwarded_immediately() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, _connection_rx, mut world_rx, _tick_tx) =
            SessionActor::for_test(key, map);

        session.handle_move_player(Direction::North).await.unwrap();

        assert!(
            matches!(world_rx.try_recv(), Ok((WorldCommand::Walk { .. }, _))),
            "nothing to wait for, so the walk goes straight through"
        );
        assert!(session.queued_walk.is_none());
    }

    /// The whole point: a walk that arrives a tick early is held, not refused.
    #[tokio::test]
    async fn a_walk_inside_the_window_is_held() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, _connection_rx, mut world_rx, _tick_tx) =
            SessionActor::for_test(key, map);
        arm_cooldown(&session, GAME_CONFIG.movement.walk_queue_ticks);

        session.handle_move_player(Direction::North).await.unwrap();

        assert!(
            world_rx.try_recv().is_err(),
            "an early walk must not reach the world, where it would be denied"
        );
        assert!(
            matches!(session.queued_walk, Some(Direction::North)),
            "it is held instead"
        );
    }

    #[tokio::test]
    async fn a_held_walk_is_forwarded_when_the_cooldown_expires() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, _connection_rx, mut world_rx, tick_tx) = SessionActor::for_test(key, map);
        arm_cooldown(&session, GAME_CONFIG.movement.walk_queue_ticks);

        session.handle_move_player(Direction::North).await.unwrap();
        session.check_queues().await.unwrap();
        assert!(world_rx.try_recv().is_err(), "still early, so still held");

        tick_tx.send(GAME_CONFIG.movement.walk_queue_ticks).unwrap();
        session.check_queues().await.unwrap();

        assert!(
            matches!(world_rx.try_recv(), Ok((WorldCommand::Walk { .. }, _))),
            "the held walk goes through on the tick the cooldown expires"
        );
        assert!(session.queued_walk.is_none(), "and the slot is cleared");
    }

    /// Newest wins. Without this a direction change inside the window would be
    /// dropped and the player would keep walking the abandoned way.
    #[tokio::test]
    async fn a_second_walk_inside_the_window_replaces_the_first() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, _connection_rx, mut world_rx, tick_tx) = SessionActor::for_test(key, map);
        arm_cooldown(&session, GAME_CONFIG.movement.walk_queue_ticks);

        session.handle_move_player(Direction::North).await.unwrap();
        session.handle_move_player(Direction::East).await.unwrap();

        tick_tx.send(GAME_CONFIG.movement.walk_queue_ticks).unwrap();
        session.check_queues().await.unwrap();

        assert!(
            matches!(
                world_rx.try_recv(),
                Ok((
                    WorldCommand::Walk {
                        direction: Direction::East,
                        ..
                    },
                    _
                ))
            ),
            "the newer direction is what fires"
        );
        assert!(
            world_rx.try_recv().is_err(),
            "and only one walk is forwarded, not two"
        );
    }

    /// Beyond the window the session does not second-guess the world. Its snapshot
    /// can be a tick stale, so refusal stays in one place.
    #[tokio::test]
    async fn a_walk_beyond_the_window_is_forwarded_for_the_world_to_deny() {
        let mut map = GameMap::new();
        let key = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, _connection_rx, mut world_rx, _tick_tx) =
            SessionActor::for_test(key, map);
        arm_cooldown(&session, GAME_CONFIG.movement.walk_queue_ticks + 1);

        session.handle_move_player(Direction::North).await.unwrap();

        assert!(
            matches!(world_rx.try_recv(), Ok((WorldCommand::Walk { .. }, _))),
            "too early to hold, so the world decides"
        );
        assert!(session.queued_walk.is_none());
    }
}
