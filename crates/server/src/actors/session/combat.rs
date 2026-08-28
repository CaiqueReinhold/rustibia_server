use anyhow::Result;

use crate::{
    actors::{session::SessionActor, world::WorldCommand},
    entities::{
        agent::{AgentId, AgentKey},
        combat::CombatDamage,
        position::Position,
        skills::SkillType,
    },
    game::combat::get_damage_visuals,
    game::skills::{progress_bp, total_experience},
    messages::{FloatingTextType, ServerMessage, SkillProgress, TextMessageType},
};

impl SessionActor {
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

    pub(super) async fn agent_took_damage(
        &self,
        agent_key: AgentKey,
        damage: CombatDamage,
    ) -> Result<()> {
        let map = self.shared_map.load();
        if agent_key == self.player_key {
            todo!();
        } else if let Some(agent_id) = self.agents.get_local(&agent_key) {
            let Some(position) = map.agent_position(agent_key) else {
                return Ok(());
            };
            let Some(agent) = map.get_agent(agent_key) else {
                return Ok(());
            };
            let (effect, text_color) = get_damage_visuals(&damage, agent);
            self.connection
                .send_message(ServerMessage::ShowEffect {
                    effect_id: effect,
                    position: position.clone(),
                    delta: Vec::new(),
                })
                .await?;
            if damage.value > 0 {
                self.connection
                    .send_message(ServerMessage::FloatingText {
                        text: damage.value.to_string(),
                        agent_id,
                        text_type: FloatingTextType::HitPoints,
                        color: Some(text_color),
                    })
                    .await?;
            }
            self.connection
                .send_message(ServerMessage::AgentLifeChanged {
                    agent_id,
                    current: agent.life().to_wire(),
                    max: 100,
                })
                .await?;
        }

        Ok(())
    }

    pub(super) async fn missile_launched(
        &self,
        from: Position,
        to: Position,
        missile_id: u16,
    ) -> Result<()> {
        self.connection
            .send_message(ServerMessage::LaunchMissile {
                from,
                to,
                missile_id,
            })
            .await?;
        Ok(())
    }

    pub(super) async fn skill_progress(&self, skill: SkillType) -> Result<()> {
        self.send_skill_update(skill).await
    }

    async fn send_skill_update(&self, skill: SkillType) -> Result<()> {
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
            self.connection
                .send_message(ServerMessage::ExperienceChanged { experience })
                .await?;
        }
        Ok(())
    }

    pub(super) async fn skill_upgraded(&self, skill: SkillType) -> Result<()> {
        self.send_skill_update(skill.clone()).await?;
        let map = self.shared_map.load();
        let message = map.get_player(self.player_key).map(|p| match skill {
            SkillType::Axe => format!("You advanced to axe fighting {}", p.skill_axe()),
            SkillType::Club => format!("You advanced to club fighting {}", p.skill_club()),
            SkillType::Sword => format!("You advanced to sword fighting {}", p.skill_sword()),
            SkillType::Distance => {
                format!("You advanced to distance fighting {}", p.skill_distance())
            }
            SkillType::Magic => format!("You advanced to magic level {}", p.skill_magic()),
            SkillType::Level => format!("You advanced to level {}", p.level()),
            SkillType::Shielding => format!("You advanced to shielding {}", p.skill_shielding()),
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
    use crate::entities::map::GameMap;
    use crate::entities::position::Position;
    use crate::entities::skills::SkillValue;
    use crate::messages::ServerMessage;

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

        session.skill_progress(SkillType::Sword).await.unwrap();

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

        session.skill_progress(SkillType::Level).await.unwrap();

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

        session.skill_progress(SkillType::Axe).await.unwrap();

        assert!(connection_rx.try_recv().is_err());
    }
}
