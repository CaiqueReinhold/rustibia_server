use anyhow::Result;

use crate::{
    actors::{session::SessionActor, world::WorldCommand},
    entities::{
        agent::{AgentId, AgentKey},
        combat::CombatDamage,
        position::Position,
    },
    game::{combat::get_damage_visuals, config::GAME_CONFIG},
    messages::{FloatingTextType, ServerMessage},
};

impl SessionActor {
    pub(super) async fn handle_set_target(
        &mut self,
        agent_id: Option<AgentId>,
        seq: u32,
    ) -> Result<()> {
        let target = agent_id.and_then(|id| self.agents.get_global(id).copied());
        self.world
            .send(WorldCommand::SetTarget {
                agent: self.player_key,
                target,
                seq,
            })
            .await;
        Ok(())
    }

    pub(super) async fn target_lost(&self, seq: u32) -> Result<()> {
        self.connection
            .send_message(ServerMessage::TargetLost { seq })
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

    pub(super) async fn potion_drunk(&self, target: AgentKey, position: Position) -> Result<()> {
        self.connection
            .send_message(ServerMessage::ShowEffect {
                effect_id: GAME_CONFIG.effect_ids.potion_use,
                position,
                delta: Vec::new(),
            })
            .await?;

        if let Some(agent_id) = self.agents.get_local(&target) {
            self.connection
                .send_message(ServerMessage::FloatingText {
                    text: "Aaaah...".to_string(),
                    agent_id,
                    text_type: FloatingTextType::CreatureSay,
                    color: None,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::connection::ConnectionCommand;
    use crate::actors::session::test_support::seat_player;
    use crate::entities::map::GameMap;

    #[tokio::test]
    async fn drinking_sends_the_effect_and_the_creature_say() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, mut connection_rx, _world_rx, _tick_tx) = SessionActor::for_test(me, map);
        session.agents.get_or_insert(me);

        session
            .potion_drunk(me, Position::new(100, 100, 7))
            .await
            .unwrap();

        let sent: Vec<_> = std::iter::from_fn(|| connection_rx.try_recv().ok()).collect();
        assert!(
            sent.iter().any(|c| matches!(
                c,
                ConnectionCommand::SendPlayerMessage(ServerMessage::ShowEffect { .. })
            )),
            "no effect was sent: {sent:?}"
        );
        assert!(
            sent.iter().any(|c| matches!(
                c,
                ConnectionCommand::SendPlayerMessage(ServerMessage::FloatingText {
                    text_type: FloatingTextType::CreatureSay,
                    ..
                })
            )),
            "no creature say was sent: {sent:?}"
        );
    }
}
