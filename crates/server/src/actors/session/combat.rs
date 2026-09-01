use anyhow::Result;

use crate::{
    actors::{session::SessionActor, world::WorldCommand},
    entities::{
        agent::{AgentId, AgentKey},
        combat::CombatDamage,
        position::Position,
    },
    game::combat::get_damage_visuals,
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
