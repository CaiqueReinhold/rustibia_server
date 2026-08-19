//! What the client currently knows about: viewport descriptions, the agent
//! id map and its recycling, spawn/despawn, and the current target.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tracing::error;

use crate::actors::player_query::get_agent_desc;
use crate::actors::player_query::get_player_desc;
use crate::actors::session::{SessionActor, SessionError};
use crate::actors::world::WorldCommand;
use crate::entities::agent::AgentId;
use crate::entities::agent::AgentKey;
use crate::entities::map::GameMap;
use crate::entities::position::Position;
use crate::game::map_query::get_agents_in_viewport;
use crate::game::map_query::get_map_desc_on_viewport;
use crate::messages::ServerMessage;
use crate::persistence::player::PlayerSnapshot;

impl SessionActor {
    pub(super) async fn player_spawned(
        &mut self,
        agent_key: AgentKey,
        position: Position,
    ) -> Result<()> {
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

    pub(super) async fn send_agents_description(
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

    pub(super) async fn send_map_description(
        &self,
        position: &Position,
        map: &GameMap,
    ) -> Result<()> {
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

    pub(super) async fn remove_agents_not_in_reach(
        &mut self,
        visible: HashSet<AgentKey>,
    ) -> Result<()> {
        let gone: Vec<AgentKey> = self
            .agents
            .iter_global()
            .filter(|key| **key != self.player_key && !visible.contains(key))
            .copied()
            .collect();

        for key in gone {
            self.forget_agent(key).await?;
        }

        Ok(())
    }

    pub(super) async fn forget_agent(&mut self, agent_key: AgentKey) -> Result<()> {
        let Some(agent_id) = self.agents.get_local(&agent_key) else {
            return Ok(());
        };

        self.agents.remove_by_local(agent_id);
        self.connection
            .send_message(ServerMessage::RemoveAgent { agent_id })
            .await?;

        self.world
            .send(WorldCommand::ClearTargetIfCurrent {
                agent: self.player_key,
                expected: agent_key,
            })
            .await;

        Ok(())
    }

    pub(super) async fn agent_despawned(
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

        self.forget_agent(agent_key).await?;
        Ok(())
    }

    pub(super) async fn handle_set_target(&mut self, agent_id: Option<AgentId>) -> Result<()> {
        let target = agent_id.and_then(|id| self.agents.get_global(id).copied());
        self.world
            .send(WorldCommand::SetTarget {
                agent: self.player_key,
                target,
            })
            .await;
        Ok(())
    }

    pub(super) async fn target_changed(&mut self, target: Option<AgentKey>) -> Result<()> {
        let agent_id = target.and_then(|key| self.agents.get_local(&key));
        self.connection
            .send_message(ServerMessage::TargetChanged { agent_id })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::connection::ConnectionCommand;
    use crate::actors::session::test_support::seat_player;
    use crate::actors::world::WorldCommand;
    use crate::entities::agent::AgentKey;
    use crate::entities::map::GameMap;
    use crate::entities::position::Position;
    use crate::game::Tick;
    use crate::messages::ServerMessage;
    use tokio::sync::mpsc;

    #[tokio::test]
    pub(super) async fn set_target_translates_the_local_id_to_a_key() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let victim = seat_player(&mut map, &Position::new(101, 100, 7), 2);
        let (mut session, _connection_rx, mut world_rx, _tick_tx) = SessionActor::for_test(me, map);
        let local = session.agents.get_or_insert(victim);

        session.handle_set_target(Some(local)).await.unwrap();

        let (cmd, _) = world_rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            WorldCommand::SetTarget { target: Some(t), .. } if t == victim
        ));
    }

    /// The agent has already left view and the client is a tick behind. The honest
    /// answer is "you have no target", not a dropped message.
    #[tokio::test]
    pub(super) async fn an_unknown_local_id_becomes_a_clear() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, _connection_rx, mut world_rx, _tick_tx) = SessionActor::for_test(me, map);

        session.handle_set_target(Some(4242)).await.unwrap();

        let (cmd, _) = world_rx.try_recv().unwrap();
        assert!(matches!(cmd, WorldCommand::SetTarget { target: None, .. }));
    }

    #[tokio::test]
    pub(super) async fn target_changed_sends_the_local_id() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let victim = seat_player(&mut map, &Position::new(101, 100, 7), 2);
        let (mut session, mut connection_rx, _world_rx, _tick_tx) = SessionActor::for_test(me, map);
        let local = session.agents.get_or_insert(victim);

        session.target_changed(Some(victim)).await.unwrap();

        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::TargetChanged { agent_id: Some(id) }
            )) if id == local
        ));
    }

    /// A target the player cannot see has no local id, and "no id" is a clear.
    #[tokio::test]
    pub(super) async fn an_unmapped_target_key_sends_a_clear() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let stranger = seat_player(&mut map, &Position::new(101, 100, 7), 2);
        let (mut session, mut connection_rx, _world_rx, _tick_tx) = SessionActor::for_test(me, map);
        // deliberately never introduced: no local id exists for `stranger`

        session.target_changed(Some(stranger)).await.unwrap();

        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::TargetChanged { agent_id: None }
            ))
        ));
    }

    /// Seats a player, a victim it is already targeting (map state kept for
    /// narrative clarity — `forget_agent` no longer reads it; the compare is the
    /// world's job now), and an unrelated bystander.
    #[allow(clippy::type_complexity)]
    pub(super) fn a_session_with_a_target() -> (
        SessionActor,
        AgentKey,
        AgentKey,
        mpsc::Receiver<ConnectionCommand>,
        mpsc::Receiver<(WorldCommand, Option<Tick>)>,
    ) {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let victim = seat_player(&mut map, &Position::new(101, 100, 7), 2);
        let bystander = seat_player(&mut map, &Position::new(102, 100, 7), 3);
        map.get_agent_mut(me).unwrap().set_target(Some(victim));

        let (session, connection_rx, world_rx, _tick_tx) = SessionActor::for_test(me, map);
        (session, victim, bystander, connection_rx, world_rx)
    }

    /// The id is recycled the moment the agent leaves view. `forget_agent` no
    /// longer decides whether this was the player's target — it just reports who
    /// left and lets the world's compare-and-swap decide, so the assertion here
    /// is on the *command shape*, not on an outcome this layer can no longer see.
    #[tokio::test]
    pub(super) async fn forgetting_a_known_agent_asks_the_world_to_clear_it_if_current() {
        let (mut session, victim, _bystander, mut connection_rx, mut world_rx) =
            a_session_with_a_target();
        session.agents.get_or_insert(victim);

        session.forget_agent(victim).await.unwrap();

        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::RemoveAgent { .. }
            ))
        ));
        let (cmd, _) = world_rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            WorldCommand::ClearTargetIfCurrent { expected, .. } if expected == victim
        ));
    }

    /// A bystander leaving still reports it as `expected` — `forget_agent` does
    /// not special-case "is this actually my target" any more (see the previous
    /// test's doc comment for why: only the world can answer that without a
    /// race). Whether the clear actually applies is `clear_target_if_current`'s
    /// job, covered in `game::targeting`'s tests, not here.
    #[tokio::test]
    pub(super) async fn forgetting_a_bystander_also_names_it_as_expected() {
        let (mut session, _victim, bystander, mut connection_rx, mut world_rx) =
            a_session_with_a_target();
        session.agents.get_or_insert(bystander);

        session.forget_agent(bystander).await.unwrap();

        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::RemoveAgent { .. }
            ))
        ));
        let (cmd, _) = world_rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            WorldCommand::ClearTargetIfCurrent { expected, .. } if expected == bystander
        ));
    }

    /// An agent that was never introduced has no id to drop and nothing to announce.
    #[tokio::test]
    pub(super) async fn forgetting_an_unknown_agent_does_nothing() {
        let (mut session, victim, _bystander, mut connection_rx, mut world_rx) =
            a_session_with_a_target();
        // `victim` is the target but was never introduced, so it has no local id.

        session.forget_agent(victim).await.unwrap();

        assert!(connection_rx.try_recv().is_err());
        assert!(world_rx.try_recv().is_err());
    }
}
