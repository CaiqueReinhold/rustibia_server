use anyhow::Result;

use crate::{
    actors::{session::SessionActor, world::WorldCommand},
    entities::{
        agent::{AgentId, AgentKey},
        combat::CombatDamage,
        creature::BloodType,
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
                agent_key: self.player_key,
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
        position: Position,
        blood_type: Option<BloodType>,
        damage: CombatDamage,
    ) -> Result<()> {
        let (effect, text_color) = get_damage_visuals(&damage, blood_type.as_ref());
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
                    position,
                    text_type: FloatingTextType::HitPoints,
                    color: Some(text_color),
                })
                .await?;
        }

        let map = self.shared_map.load();
        let Some(agent_id) = self.agents.get_local(&agent_key) else {
            return Ok(());
        };
        let Some(agent) = map.get_agent(agent_key) else {
            // agent died and got removed, no need to send life update
            return Ok(());
        };
        let (current, max) = if agent_key == self.player_key {
            (agent.life().current, agent.life().maximum)
        } else {
            (agent.life().to_wire(), 100)
        };

        self.connection
            .send_message(ServerMessage::AgentLifeChanged {
                agent_id,
                current,
                max,
            })
            .await?;

        Ok(())
    }

    pub(super) async fn potion_drunk(&self, target: AgentKey, position: Position) -> Result<()> {
        self.connection
            .send_message(ServerMessage::ShowEffect {
                effect_id: GAME_CONFIG.effect_ids.potion_use,
                position: position.clone(),
                delta: Vec::new(),
            })
            .await?;

        // The text carries no agent, but it is still only for a drinker
        // this session has told the client about.
        if self.agents.get_local(&target).is_some() {
            self.connection
                .send_message(ServerMessage::FloatingText {
                    text: "Aaaah...".to_string(),
                    position,
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
    use crate::entities::combat::CombatElement;
    use crate::entities::map::GameMap;

    /// The killing blow. `game::damage::apply_damage` reaps its target inside the
    /// tick that produced this message, so the agent is already gone from the
    /// snapshot this session reads -- and the number and the splash must still go
    /// out, pinned to the tile the message carried. Asking the map for the target
    /// here is what used to drop both, making every kill look like an animation
    /// cut short.
    #[tokio::test]
    async fn a_hit_on_an_agent_the_map_has_already_reaped_still_draws() {
        let mut map = GameMap::new();
        let me = seat_player(&mut map, &Position::new(100, 100, 7), 1);
        let (mut session, mut connection_rx, _world_rx, _tick_tx) = SessionActor::for_test(me, map);
        // Known to the session, absent from the map: exactly what a reaped
        // creature looks like for the rest of the batch that killed it.
        let reaped = AgentKey::default();
        session.agents.get_or_insert(reaped);
        let tile = Position::new(101, 100, 7);

        session
            .agent_took_damage(
                reaped,
                tile.clone(),
                Some(BloodType::Blood),
                CombatDamage {
                    element: CombatElement::Physical,
                    value: 30,
                    blocked_shield: false,
                    blocked_armor: false,
                },
            )
            .await
            .unwrap();

        let sent: Vec<_> = std::iter::from_fn(|| connection_rx.try_recv().ok()).collect();
        assert!(
            sent.iter().any(|c| matches!(
                c,
                ConnectionCommand::SendPlayerMessage(ServerMessage::ShowEffect { .. })
            )),
            "the hit splash was dropped: {sent:?}"
        );
        let number = sent
            .iter()
            .find_map(|c| match c {
                ConnectionCommand::SendPlayerMessage(ServerMessage::FloatingText {
                    text,
                    position,
                    text_type: FloatingTextType::HitPoints,
                    ..
                }) => Some((text.clone(), position.clone())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the damage number was dropped: {sent:?}"));
        assert_eq!(number, ("30".to_owned(), tile));
    }

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
