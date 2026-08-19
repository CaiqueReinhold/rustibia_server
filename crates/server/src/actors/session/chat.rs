//! Chat: local speech, private messages, channels, and the session-local
//! chat ids that name their authors over the wire.

use anyhow::Result;

use crate::actors::session::SessionActor;
use crate::actors::world::WorldCommand;
use crate::entities::agent::AgentId;
use crate::entities::agent::AgentKey;
use crate::entities::chat::ChannelId;
use crate::entities::chat::ChatMessageType;
use crate::game::game_config::GAME_CONFIG;
use crate::messages::ServerMessage;
use crate::messages::TextMessageType;

impl SessionActor {
    pub(super) async fn introduce(&mut self, agent_key: AgentKey) -> Result<Option<AgentId>> {
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

    pub(super) async fn deny(&self, text: &str) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TextMessage {
                text: text.to_owned(),
                message_type: TextMessageType::ActionDenied,
            })
            .await?;
        Ok(())
    }

    pub(super) async fn send_chat(
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

    pub(super) async fn receive_private_message(
        &mut self,
        author: AgentKey,
        message: String,
    ) -> Result<()> {
        self.send_chat(author, ChatMessageType::Private, 0, message)
            .await
    }

    pub(super) async fn receive_channel_message(
        &mut self,
        author: AgentKey,
        channel: ChannelId,
        message: String,
    ) -> Result<()> {
        self.send_chat(author, ChatMessageType::Channel, channel, message)
            .await
    }

    pub(super) async fn handle_say(
        &mut self,
        message: String,
        message_type: ChatMessageType,
        target: u16,
    ) -> Result<()> {
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

    pub(super) async fn handle_open_pm_chat(&mut self, name: String) -> Result<()> {
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

    pub(super) async fn handle_request_channels(&self) -> Result<()> {
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

    pub(super) async fn handle_open_channel(&self, channel: ChannelId) -> Result<()> {
        self.chat.join_channel(self.player_key, channel).await;
        Ok(())
    }

    pub(super) async fn handle_close_channel(&self, channel: ChannelId) -> Result<()> {
        self.chat.leave_channel(self.player_key, channel).await;
        Ok(())
    }

    pub(super) async fn agent_said(&mut self, agent_key: AgentKey, message: String) -> Result<()> {
        self.send_chat(agent_key, ChatMessageType::Local, 0, message)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::connection::ConnectionCommand;
    use crate::actors::session::test_support::seat_player;
    use crate::entities::map::GameMap;
    use crate::entities::position::Position;

    #[tokio::test]
    pub(super) async fn an_author_is_introduced_exactly_once() {
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
    pub(super) async fn an_author_no_longer_on_the_map_is_not_introduced() {
        let (mut session, _connection_rx, _world_rx, _tick_tx) =
            SessionActor::for_test(AgentKey::default(), GameMap::new());

        assert_eq!(session.introduce(AgentKey::default()).await.unwrap(), None);
    }

    #[tokio::test]
    pub(super) async fn an_over_length_message_is_denied() {
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
    pub(super) async fn the_length_limit_counts_characters_not_bytes() {
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
    pub(super) async fn a_second_message_inside_the_cooldown_is_dropped() {
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
    pub(super) async fn opening_a_pm_chat_with_an_offline_name_is_denied() {
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
    pub(super) async fn opening_a_pm_chat_introduces_the_target() {
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
}
