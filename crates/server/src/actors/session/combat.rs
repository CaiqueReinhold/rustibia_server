use anyhow::Result;
use tracing::info;

use crate::{
    actors::{session::SessionActor, world::WorldCommand},
    entities::{
        agent::{AgentId, AgentKey},
        combat::CombatDamage,
        position::Position,
        skills::SkillType,
    },
    game::combat::get_damage_visuals,
    messages::{FloatingTextType, ServerMessage},
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
        self.has_target = target.is_some();
        let agent_id = target.and_then(|key| self.agents.get_local(&key));
        self.connection
            .send_message(ServerMessage::TargetChanged { agent_id })
            .await?;
        Ok(())
    }

    pub(super) async fn check_auto_attack(&self) -> Result<()> {
        if !self.has_target {
            return Ok(());
        }

        info!("checking auto attack");

        let map = self.shared_map.load();
        let remaining_ticks = map
            .get_agent(self.player_key)
            .map(|p| p.next_attack_tick.saturating_sub(*self.tick_rx.borrow()))
            .unwrap_or(0);

        info!("remaining ticks {}", remaining_ticks);

        if remaining_ticks > 1 {
            return Ok(());
        }

        self.world
            .send(WorldCommand::AutoAttackTarget {
                agent: self.player_key,
            })
            .await;

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
            self.connection
                .send_message(ServerMessage::FloatingText {
                    text: damage.value.to_string(),
                    agent_id,
                    text_type: FloatingTextType::HitPoints,
                    color: Some(text_color),
                })
                .await?;
            self.connection
                .send_message(ServerMessage::AgentLifeChanged {
                    agent_id,
                    life: agent.life().to_wire(),
                })
                .await?;
        }

        Ok(())
    }

    pub(super) async fn missile_launched(
        &self,
        from: Position,
        to: Position,
        sprite_id: u16,
    ) -> Result<()> {
        Ok(())
    }

    pub(super) async fn skill_progress(&self, skill: SkillType) -> Result<()> {
        Ok(())
    }

    pub(super) async fn skill_upgraded(&self, skill: SkillType) -> Result<()> {
        Ok(())
    }
}
