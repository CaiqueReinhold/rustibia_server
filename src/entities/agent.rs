use slotmap::new_key_type;

use super::player::Player;
use crate::{actors::Tick, entities::map::Position};

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
    inner: AgentInner,
}

impl Agent {
    pub fn from_player(player: Player) -> Self {
        Self {
            handle: None,
            inner: AgentInner::Player(player),
            next_walk_tick: 0,
        }
    }

    pub fn position(&self) -> &Position {
        match &self.inner {
            AgentInner::Player(player) => &player.position,
            AgentInner::Creature => todo!(),
        }
    }

    pub fn set_position(&mut self, pos: Position) {
        match &mut self.inner {
            AgentInner::Player(player) => player.position = pos,
            AgentInner::Creature => todo!(),
        }
    }

    pub fn calculate_walk_ticks(&self, _tile_speed: u8) -> Tick {
        todo!()
    }
}
