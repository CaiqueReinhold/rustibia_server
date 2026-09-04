pub mod admin;
pub mod chat;
pub mod combat;
pub mod config;
pub mod creature_behavior;
pub mod damage;
pub mod death;
pub mod description;
pub mod events;
pub mod experience;
pub mod item_action;
pub mod item_movement;
pub mod item_multi_action;
pub mod map_query;
pub mod movement;
pub mod pathfinding;
pub mod random;
pub mod skills;
pub mod targeting;

use crate::actors::world::ScheduledCommand;
use crate::entities::map::GameMap;
use crate::game::events::BroadcastMessage;
use crate::game::random::Rolls;

pub type Tick = u64;

pub struct TickCtx<'a> {
    pub map: &'a mut GameMap,
    pub events: &'a mut Vec<BroadcastMessage>,
    pub scheduled: &'a mut Vec<ScheduledCommand>,
    pub roll: &'a mut Rolls,
    pub tick: Tick,
}

#[derive(Clone, Copy)]
pub struct Mark {
    events: usize,
    scheduled: usize,
}

impl TickCtx<'_> {
    pub fn mark(&self) -> Mark {
        Mark {
            events: self.events.len(),
            scheduled: self.scheduled.len(),
        }
    }

    /// Discards everything reported since `mark`.
    pub fn rollback_to(&mut self, mark: Mark) {
        self.events.truncate(mark.events);
        self.scheduled.truncate(mark.scheduled);
    }
}

#[cfg(test)]
pub struct TestHarness {
    pub events: Vec<BroadcastMessage>,
    pub scheduled: Vec<ScheduledCommand>,
    pub roll: Rolls,
    pub tick: Tick,
}

#[cfg(test)]
impl TestHarness {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            scheduled: Vec::new(),
            roll: Rolls::new(0),
            tick: 0,
        }
    }

    pub fn seeded(seed: u64) -> Self {
        Self {
            roll: Rolls::new(seed),
            ..Self::new()
        }
    }

    pub fn ctx<'a>(&'a mut self, map: &'a mut GameMap) -> TickCtx<'a> {
        TickCtx {
            map,
            events: &mut self.events,
            scheduled: &mut self.scheduled,
            roll: &mut self.roll,
            tick: self.tick,
        }
    }
}
