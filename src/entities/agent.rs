use std::collections::HashMap;

use slotmap::new_key_type;

use super::{inventory::Inventory, player::Player};
use crate::{
    config,
    constants::{SPEED_PARAM_A, SPEED_PARAM_B, SPEED_PARAM_C},
    entities::skills::{SkillType, SkillValue},
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

    pub fn from_player(player: PlayerSnapshot) -> Self {
        let mut agent = Self {
            inner: AgentInner::Player(Player {
                id: player.id,
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
        };
        agent.apply_modifiers();
        agent
    }

    // Testing
    pub fn new_creature() -> Self {
        let mut creature = Self {
            name: "Demon".to_string(),
            facing: Facing::South,
            life: Pool {
                current: 100,
                maximum: 100,
            },
            outfit: (35, (0, 0, 0, 0)),
            skills: HashMap::new(),
            inner: AgentInner::Creature,
            speed: 200,
            next_use_tick: 0,
            next_walk_tick: 0,
        };
        creature.skills.insert(
            SkillType::Level,
            SkillValue {
                value: 1,
                current_ticks: 0,
                max_ticks: 100,
            },
        );
        creature.skills.insert(
            SkillType::Speed,
            SkillValue {
                value: 120,
                current_ticks: 0,
                max_ticks: 0,
            },
        );
        creature
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
}
