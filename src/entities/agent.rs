use std::collections::HashMap;

use slotmap::new_key_type;

use super::{inventory::Inventory, player::Player};
use crate::{
    config,
    constants::{SPEED_PARAM_A, SPEED_PARAM_B, SPEED_PARAM_C},
    entities::{
        creature::CreatureKind,
        position::Position,
        skills::{SkillType, SkillValue},
    },
    game::Tick,
    persistence::player::PlayerSnapshot,
};

pub type AgentId = u16;
pub type OutfitId = u16;
pub type OutfitColors = (u8, u8, u8, u8);

#[derive(Clone, Debug)]
pub struct Pool {
    pub current: u32,
    pub maximum: u32,
}

impl Pool {
    pub fn available(&self) -> u32 {
        self.maximum - self.current
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    North,
    East,
    South,
    West,
}

#[derive(Clone, Debug)]
enum AgentInner {
    Player(Player),
    Creature,
}

new_key_type! { pub struct AgentKey; }

#[derive(Clone, Debug)]
pub struct Agent {
    inner: AgentInner,
    name: String,
    life: Pool,
    skills: HashMap<SkillType, SkillValue>,
    outfit: (OutfitId, OutfitColors),
    speed: u16,

    pub facing: Facing,
    pub next_walk_tick: Tick,
    pub next_use_tick: Tick,

    pub next_wander_tick: Tick,
}

impl Agent {
    pub fn get_player(&self) -> Option<&Player> {
        match &self.inner {
            AgentInner::Player(p) => Some(p),
            AgentInner::Creature => None,
        }
    }

    pub fn get_player_mut(&mut self) -> Option<&mut Player> {
        match &mut self.inner {
            AgentInner::Player(p) => Some(p),
            AgentInner::Creature => None,
        }
    }

    pub fn is_creature(&self) -> bool {
        matches!(self.inner, AgentInner::Creature)
    }

    pub fn from_player(player: PlayerSnapshot) -> Self {
        let mut agent = Self {
            inner: AgentInner::Player(Player {
                id: player.id,
                account_id: player.account_id,
                position: player.position,
                origin: player.origin,
                mana: player.mana,
                capacity: player.capacity,
                inventory: Inventory::from_snapshot(player.inventory),
            }),
            name: player.name,
            facing: player.facing,
            life: player.life,
            skills: player.skills,
            outfit: player.outfit,
            speed: 0,
            next_walk_tick: 0,
            next_use_tick: 0,
            next_wander_tick: 0,
        };
        agent.apply_modifiers();
        agent
    }

    pub fn from_creature_kind(kind: &CreatureKind) -> Self {
        let mut agent = Self {
            inner: AgentInner::Creature,
            name: kind.name.clone(),
            life: kind.life.clone(),
            skills: kind.skills.clone(),
            outfit: kind.outfit,
            speed: kind.speed,
            facing: Facing::South,
            next_walk_tick: 0,
            next_use_tick: 0,
            next_wander_tick: 0,
        };
        agent.apply_modifiers();
        agent
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn life(&self) -> &Pool {
        &self.life
    }

    pub fn outfit(&self) -> (OutfitId, OutfitColors) {
        self.outfit
    }

    pub fn speed(&self) -> u16 {
        self.speed
    }

    pub fn get_skill(&self, skill: SkillType) -> Option<&SkillValue> {
        self.skills.get(&skill)
    }

    pub fn apply_modifiers(&mut self) {
        if let Some(speed) = self.skills.get(&SkillType::Speed) {
            // TODO: apply effects
            self.speed = speed.value;
        }
    }

    pub fn calculate_walk_ticks(&self, tile_friction: u32, diagonal: bool) -> Tick {
        let move_speed = (SPEED_PARAM_A * ((self.speed as f32) + SPEED_PARAM_B).ln()
            + SPEED_PARAM_C)
            .round()
            .max(1.0);

        let mut tile_speed = (1000.0 * (tile_friction as f32) / move_speed).floor();
        if diagonal {
            tile_speed *= 2.5;
        }

        (tile_speed / (config::CONFIG.tick_duration.as_millis() as f32)).ceil() as Tick
    }

    pub fn can_logout(&self, current_tick: Tick) -> bool {
        self.next_walk_tick <= current_tick
    }

    pub fn to_snapshot(&self, position: Position) -> Option<PlayerSnapshot> {
        let player = self.get_player()?;
        Some(PlayerSnapshot {
            id: player.id,
            account_id: player.account_id,
            name: self.name.clone(),
            position,
            origin: player.origin.clone(),
            facing: self.facing,
            life: self.life.clone(),
            mana: player.mana.clone(),
            capacity: player.capacity.clone(),
            outfit: self.outfit,
            skills: self.skills.clone(),
            inventory: player.inventory.slots().clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::position::Position;
    use crate::entities::skills::{SkillType, SkillValue};
    use crate::persistence::player::PlayerSnapshot;
    use std::collections::HashMap;

    fn make_snapshot(id: u32) -> PlayerSnapshot {
        PlayerSnapshot {
            id,
            account_id: 1,
            name: "Rizael".to_string(),
            position: Position {
                x: 100,
                y: 100,
                z: 7,
            },
            origin: Position {
                x: 100,
                y: 100,
                z: 7,
            },
            facing: Facing::North,
            life: Pool {
                current: 80,
                maximum: 100,
            },
            mana: Pool {
                current: 50,
                maximum: 100,
            },
            capacity: Pool {
                current: 0,
                maximum: 40000,
            },
            outfit: (133, (1, 2, 3, 4)),
            skills: {
                let mut m = HashMap::new();
                m.insert(
                    SkillType::Speed,
                    SkillValue {
                        value: 120,
                        current_ticks: 0,
                        max_ticks: 0,
                    },
                );
                m
            },
            inventory: HashMap::new(),
        }
    }

    #[test]
    fn to_snapshot_returns_none_for_creature() {
        let creature = Agent::from_creature_kind(&CreatureKind {
            name: "Creature".to_string(),
            life: Pool {
                current: 1,
                maximum: 1,
            },
            outfit: (1, (0, 0, 0, 0)),
            speed: 1,
            skills: HashMap::new(),
        });
        let pos = Position {
            x: 200,
            y: 200,
            z: 7,
        };
        assert!(creature.to_snapshot(pos).is_none());
    }

    #[test]
    fn to_snapshot_uses_passed_position_not_stored() {
        let agent = Agent::from_player(make_snapshot(1));
        let new_pos = Position {
            x: 999,
            y: 888,
            z: 5,
        };
        let snap = agent.to_snapshot(new_pos.clone()).unwrap();
        assert_eq!(snap.position, new_pos);
        assert_eq!(snap.id, 1);
        assert_eq!(snap.name, "Rizael");
        assert_eq!(snap.facing, Facing::North);
        assert_eq!(snap.life.current, 80);
        assert_eq!(snap.life.maximum, 100);
        assert_eq!(snap.mana.current, 50);
        assert_eq!(snap.capacity.maximum, 40000);
        assert_eq!(snap.outfit, (133, (1, 2, 3, 4)));
        assert_eq!(snap.skills[&SkillType::Speed].value, 120);
    }

    #[test]
    fn can_logout_when_walk_tick_is_current_or_past() {
        let agent = Agent::from_player(make_snapshot(1));
        // next_walk_tick defaults to 0
        assert!(agent.can_logout(0));
        assert!(agent.can_logout(1));
    }

    #[test]
    fn cannot_logout_when_walk_tick_is_in_future() {
        let mut agent = Agent::from_player(make_snapshot(1));
        agent.next_walk_tick = 10;
        assert!(!agent.can_logout(9));
        assert!(agent.can_logout(10));
        assert!(agent.can_logout(11));
    }

    #[test]
    fn is_creature_distinguishes_player_and_creature() {
        let player = Agent::from_player(make_snapshot(1));
        let creature = Agent::from_creature_kind(&CreatureKind {
            name: "Creature".to_string(),
            life: Pool {
                current: 1,
                maximum: 1,
            },
            outfit: (1, (0, 0, 0, 0)),
            speed: 1,
            skills: HashMap::new(),
        });
        assert!(!player.is_creature());
        assert!(creature.is_creature());
    }

    #[test]
    fn from_creature_kind_produces_creature_agent_with_kind_attributes() {
        use crate::entities::creature::CreatureKind;
        let kind = CreatureKind {
            name: "Demon".to_string(),
            life: Pool {
                current: 8200,
                maximum: 8200,
            },
            outfit: (35, (0, 0, 0, 0)),
            speed: 230,
            skills: HashMap::new(),
        };
        let agent = Agent::from_creature_kind(&kind);
        assert!(agent.is_creature());
        assert_eq!(agent.name(), "Demon");
        assert_eq!(agent.life().maximum, 8200);
        assert_eq!(agent.outfit(), (35, (0, 0, 0, 0)));
    }
}
