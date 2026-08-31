//! What the client currently knows about: viewport descriptions, the agent
//! id map and its recycling, spawn/despawn, and the current target.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tracing::error;

use crate::actors::player_query::{get_agent_desc, get_player_desc, get_player_skills};
use crate::actors::session::{SessionActor, SessionError};
use crate::actors::world::WorldCommand;
use crate::entities::agent::AgentKey;
use crate::entities::map::GameMap;
use crate::entities::position::Position;
use crate::entities::skills::SkillType;
use crate::game::config::GAME_CONFIG;
use crate::game::map_query::get_agents_in_viewport;
use crate::game::map_query::get_map_desc_on_viewport;
use crate::game::skills::{progress_bp, total_experience};
use crate::messages::{FloatingTextType, ServerMessage, SkillProgress, TextMessageType};
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

            if let Some(skills_msg) = get_player_skills(&map, self.player_key) {
                self.connection.send_message(skills_msg).await?;
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

    pub(super) async fn mana_updated(&self) -> Result<()> {
        let map = self.shared_map.load();
        if let Some(player) = map.get_player(self.player_key)
            && let Some(agent_id) = self.agents.get_local(&self.player_key)
        {
            self.connection
                .send_message(ServerMessage::AgentManaChanged {
                    agent_id,
                    current: player.mana.current,
                    max: player.mana.maximum,
                })
                .await?;
        }
        Ok(())
    }

    pub(super) async fn skill_progress(&self, skill: SkillType, amount: u64) -> Result<()> {
        self.send_skill_update(skill, amount).await
    }

    async fn send_skill_update(&self, skill: SkillType, amount: u64) -> Result<()> {
        let (progress, experience) = {
            let map = self.shared_map.load();
            let Some(player) = map.get_player(self.player_key) else {
                return Ok(());
            };
            let Some(value) = player.skills.get(&skill) else {
                return Ok(());
            };
            (
                SkillProgress {
                    level: value.value,
                    percent_bp: progress_bp(player.vocation, &skill, value),
                },
                (skill == SkillType::Level).then(|| total_experience(value)),
            )
        };

        self.connection
            .send_message(ServerMessage::SkillChanged { skill, progress })
            .await?;

        if let Some(experience) = experience {
            if let Some(agent_id) = self.agents.get_local(&self.player_key) {
                self.connection
                    .send_message(ServerMessage::FloatingText {
                        text: amount.to_string(),
                        agent_id,
                        text_type: FloatingTextType::HitPoints,
                        color: Some(GAME_CONFIG.text_colors.white),
                    })
                    .await?;
            }
            self.connection
                .send_message(ServerMessage::ExperienceChanged { experience })
                .await?;
        }
        Ok(())
    }

    pub(super) async fn skill_upgraded(&self, skill: SkillType, gained: u16) -> Result<()> {
        self.send_skill_update(skill.clone(), 0).await?;
        let map = self.shared_map.load();
        let message = map.get_player(self.player_key).map(|p| match skill {
            SkillType::Axe => format!("You advanced to axe fighting {}", p.skill_axe()),
            SkillType::Club => format!("You advanced to club fighting {}", p.skill_club()),
            SkillType::Sword => format!("You advanced to sword fighting {}", p.skill_sword()),
            SkillType::Distance => {
                format!("You advanced to distance fighting {}", p.skill_distance())
            }
            SkillType::Magic => format!("You advanced to magic level {}", p.skill_magic()),
            SkillType::Shielding => format!("You advanced to shielding {}", p.skill_shielding()),
            SkillType::Level => format!(
                "You advanced from level {} to level {}",
                p.level().saturating_sub(gained),
                p.level()
            ),
        });

        if let Some(message) = message {
            self.connection
                .send_message(ServerMessage::TextMessage {
                    text: message,
                    message_type: TextMessageType::Look,
                })
                .await?;
        }
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
    use crate::entities::skills::SkillValue;
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

    #[tokio::test]
    async fn a_ticked_skill_sends_its_level_and_progress() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        map.get_player_mut(me).unwrap().skills.insert(
            SkillType::Sword,
            SkillValue {
                value: 11,
                current_ticks: 27,
            },
        );
        let (session, mut connection_rx, _world_rx, _tick_tx) = SessionActor::for_test(me, map);

        session.skill_progress(SkillType::Sword, 1).await.unwrap();

        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::SkillChanged {
                    skill: SkillType::Sword,
                    progress,
                }
            )) if progress.level == 11 && progress.percent_bp == 4909
        ));
    }

    /// Experience is its own message, so a client that only wants the total does
    /// not have to know that `Level` is a skill.
    #[tokio::test]
    async fn the_level_skill_also_sends_the_experience_total() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        map.get_player_mut(me).unwrap().skills.insert(
            SkillType::Level,
            SkillValue {
                value: 8,
                current_ticks: 55,
            },
        );
        let (session, mut connection_rx, _world_rx, _tick_tx) = SessionActor::for_test(me, map);

        session.skill_progress(SkillType::Level, 30).await.unwrap();

        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::SkillChanged { .. }
            ))
        ));
        assert!(matches!(
            connection_rx.try_recv(),
            Ok(ConnectionCommand::SendPlayerMessage(
                ServerMessage::ExperienceChanged { experience: 4255 }
            ))
        ));
    }

    /// `tick_skill` returns without emitting for a skill the player has no row
    /// for, but the broadcast handler must not assume that — the map it reads is
    /// a snapshot, not the map the event was produced from.
    #[tokio::test]
    async fn a_skill_the_player_does_not_have_sends_nothing() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        map.get_player_mut(me).unwrap().skills.clear();
        let (session, mut connection_rx, _world_rx, _tick_tx) = SessionActor::for_test(me, map);

        session.skill_progress(SkillType::Axe, 1).await.unwrap();

        assert!(connection_rx.try_recv().is_err());
    }
}
