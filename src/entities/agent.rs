use slotmap::new_key_type;

use super::player::Player;
use crate::{
    actors::Tick,
    config,
    constants::{SPEED_PARAM_A, SPEED_PARAM_B, SPEED_PARAM_C},
};

#[derive(Clone, Debug)]
enum AgentInner {
    Player(Player),
    Creature,
}

new_key_type! { pub struct AgentKey; }

#[derive(Clone, Debug)]
pub struct Agent {
    pub handle: Option<AgentKey>,
    pub next_walk_tick: Tick,
    pub speed: u16,
    inner: AgentInner,
}

impl Agent {
    pub fn get_player(&self) -> Option<&Player> {
        match &self.inner {
            AgentInner::Player(p) => Some(p),
            AgentInner::Creature => None,
        }
    }

    pub fn from_player(player: Player) -> Self {
        let speed = player.base_speed;
        Self {
            handle: None,
            inner: AgentInner::Player(player),
            next_walk_tick: 0,
            speed,
        }
    }

    pub fn calculate_walk_ticks(&self, tile_friction: u32, diagonal: bool) -> Tick {
        let move_speed = (SPEED_PARAM_A * ((self.speed as f32) + SPEED_PARAM_B).ln()
            + SPEED_PARAM_C)
            .round()
            .max(1.0);

        let mut tile_speed = (1000.0 * (tile_friction as f32) / move_speed).floor();
        if diagonal {
            tile_speed *= 2.0;
        }

        (tile_speed / (config::CONFIG.tick_duration.as_millis() as f32)).ceil() as Tick
    }
}
