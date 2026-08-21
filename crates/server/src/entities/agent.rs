use std::sync::Arc;

use slotmap::new_key_type;

use super::{inventory::Inventory, player::Player};
use crate::{
    config,
    constants::{SPEED_PARAM_A, SPEED_PARAM_B, SPEED_PARAM_C},
    entities::{creature::CreatureKind, position::Position},
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

    pub fn remove(&mut self, amount: u32) {
        self.current = self.current.saturating_sub(amount)
    }

    pub fn to_wire(&self) -> u8 {
        ((self.current as f32) / (self.maximum as f32) * 100.0).round() as u8
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
    Creature(Arc<CreatureKind>),
}

#[derive(Debug, Clone, Default)]
struct Modifiers {
    speed: u16,
}

new_key_type! { pub struct AgentKey; }

#[derive(Clone, Debug)]
pub struct Agent {
    inner: AgentInner,
    life: Pool,
    outfit: (OutfitId, OutfitColors),
    base_speed: u16,
    modifiers: Modifiers,
    facing: Facing,

    // both
    pub next_walk_tick: Tick,
    pub next_attack_tick: Tick,

    // player
    pub next_use_tick: Tick,

    // creature
    pub next_wander_tick: Tick,

    target: Option<AgentKey>,
}

impl Agent {
    pub fn get_player(&self) -> Option<&Player> {
        match &self.inner {
            AgentInner::Player(p) => Some(p),
            AgentInner::Creature(..) => None,
        }
    }

    pub fn get_player_mut(&mut self) -> Option<&mut Player> {
        match &mut self.inner {
            AgentInner::Player(p) => Some(p),
            AgentInner::Creature(..) => None,
        }
    }

    pub fn get_creature_kind(&self) -> Option<&CreatureKind> {
        match &self.inner {
            AgentInner::Creature(c) => Some(c),
            AgentInner::Player(..) => None,
        }
    }

    pub fn is_creature(&self) -> bool {
        matches!(self.inner, AgentInner::Creature(..))
    }

    pub fn from_player(player: PlayerSnapshot) -> Self {
        let inventory = Inventory::from_snapshot(player.inventory);
        Self {
            inner: AgentInner::Player(Player {
                id: player.id,
                name: player.name,
                account_id: player.account_id,
                position: player.position,
                origin: player.origin,
                mana: player.mana,
                capacity: Pool {
                    current: inventory.total_weight(),
                    maximum: player.capacity,
                },
                inventory,
                skills: player.skills,
            }),
            facing: player.facing,
            life: player.life,
            outfit: player.outfit,
            base_speed: player.speed,
            next_walk_tick: 0,
            next_use_tick: 0,
            next_wander_tick: 0,
            next_attack_tick: 0,
            target: None,
            modifiers: Modifiers::default(),
        }
    }

    pub fn from_creature_kind(kind: Arc<CreatureKind>) -> Self {
        let life = kind.life.clone();
        let outfit = kind.outfit;
        let speed = kind.speed;
        Self {
            inner: AgentInner::Creature(kind),
            life,
            outfit,
            base_speed: speed,
            facing: Facing::South,
            next_walk_tick: 0,
            next_use_tick: 0,
            next_wander_tick: 0,
            next_attack_tick: 0,
            target: None,
            modifiers: Modifiers::default(),
        }
    }

    pub fn name(&self) -> &str {
        match &self.inner {
            AgentInner::Creature(c) => &c.name,
            AgentInner::Player(p) => &p.name,
        }
    }

    pub fn life(&self) -> &Pool {
        &self.life
    }

    pub fn take_hit(&mut self, damage: u32, _attacker: Option<AgentKey>) {
        self.life.current = self.life.current.saturating_sub(damage);
    }

    pub fn outfit(&self) -> (OutfitId, OutfitColors) {
        self.outfit
    }

    pub fn speed(&self) -> u16 {
        self.base_speed + self.modifiers.speed
    }

    pub fn facing(&self) -> Facing {
        self.facing
    }

    pub fn set_facing(&mut self, facing: Facing) {
        self.facing = facing;
    }

    pub fn target(&self) -> Option<AgentKey> {
        self.target
    }

    pub fn set_target(&mut self, target: Option<AgentKey>) {
        self.target = target;
    }

    pub fn calculate_walk_ticks(&self, tile_friction: u16, diagonal: bool) -> Tick {
        let move_speed = (SPEED_PARAM_A * ((self.speed() as f32) + SPEED_PARAM_B).ln()
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

    pub fn attack_range(&self) -> u8 {
        match &self.inner {
            AgentInner::Player(p) => p.weapon_range(),
            AgentInner::Creature(..) => 1,
        }
    }

    pub fn to_snapshot(&self, position: Position) -> Option<PlayerSnapshot> {
        let player = self.get_player()?;
        Some(PlayerSnapshot {
            id: player.id,
            account_id: player.account_id,
            name: player.name.clone(),
            position,
            origin: player.origin.clone(),
            facing: self.facing,
            life: self.life.clone(),
            mana: player.mana.clone(),
            capacity: player.capacity.maximum,
            speed: self.base_speed,
            outfit: self.outfit,
            skills: player.skills.clone(),
            inventory: player.inventory.slots().clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::creature::BloodType;
    use crate::entities::map::GameMap;
    use crate::entities::position::Position;
    use crate::entities::skills::{SkillType, SkillValue};
    use crate::persistence::player::PlayerSnapshot;
    use crate::persistence::test_fixtures::a_test_snapshot;
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
            capacity: 40000,
            speed: 100,
            outfit: (133, (1, 2, 3, 4)),
            skills: {
                let mut m = HashMap::new();
                m.insert(
                    SkillType::Level,
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
        let creature = Agent::from_creature_kind(Arc::new(CreatureKind {
            name: "Creature".to_string(),
            life: Pool {
                current: 1,
                maximum: 1,
            },
            speed: 1,
            auto_attack_damage: (1, 2),
            outfit: (1, (0, 0, 0, 0)),
            blood_type: BloodType::Blood,
        }));
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
        assert_eq!(snap.capacity, 40000);
        assert_eq!(snap.outfit, (133, (1, 2, 3, 4)));
        assert_eq!(snap.skills[&SkillType::Level].value, 120);
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
        let creature = Agent::from_creature_kind(Arc::new(CreatureKind {
            name: "Creature".to_string(),
            life: Pool {
                current: 1,
                maximum: 1,
            },
            auto_attack_damage: (1, 2),
            outfit: (1, (0, 0, 0, 0)),
            speed: 1,
            blood_type: BloodType::Blood,
        }));
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
            auto_attack_damage: (1, 2),
            outfit: (35, (0, 0, 0, 0)),
            speed: 1,
            blood_type: BloodType::Blood,
        };
        let agent = Agent::from_creature_kind(Arc::new(kind));
        assert!(agent.is_creature());
        assert_eq!(agent.name(), "Demon");
        assert_eq!(agent.life().maximum, 8200);
        assert_eq!(agent.outfit(), (35, (0, 0, 0, 0)));
    }

    /// Paired with `step_duration_matches_the_server` in the client's
    /// `agent/components.rs`. The two formulas are deliberate duplicates across
    /// two repositories: a divergence compiles cleanly on both sides and simply
    /// desyncs movement, so each side pins the same three answers.
    ///
    /// The client asserts milliseconds; these are the same numbers divided by the
    /// 50ms tick. `a_test_snapshot` gives the agent a speed of 120, which is the
    /// speed the client's paired test uses — moving that fixture desyncs the pair
    /// without either side failing to compile.
    #[test]
    fn walk_ticks_match_the_client() {
        let agent = Agent::from_player(a_test_snapshot(1, 1));
        assert_eq!(
            agent.speed(),
            120,
            "the fixture's speed column feeds the formula"
        );

        assert_eq!(agent.calculate_walk_ticks(150, false), 10, "500ms");
        assert_eq!(agent.calculate_walk_ticks(150, true), 25, "1250ms diagonal");
        // 260 is the friction of `ornamented stone floor` (id 21718), one of the
        // ten values the client used to truncate through a `u8`.
        assert_eq!(agent.calculate_walk_ticks(260, false), 18, "900ms");
    }

    #[test]
    fn a_new_agent_has_no_target() {
        let agent = Agent::from_player(make_snapshot(1));
        assert!(agent.target().is_none());
    }

    #[test]
    fn set_target_stores_and_clears() {
        let mut agent = Agent::from_player(make_snapshot(1));
        let mut map = GameMap::new();
        map.insert_tile(Position::new(1, 1, 7), crate::entities::map::MapTile::new());
        let victim = map
            .insert_agent(
                Agent::from_player(make_snapshot(2)),
                &Position::new(1, 1, 7),
            )
            .unwrap();

        agent.set_target(Some(victim));
        assert_eq!(agent.target(), Some(victim));

        agent.set_target(None);
        assert!(agent.target().is_none());
    }

    /// The target is session state. A character that logs out and back in must not
    /// come back still targeting something.
    #[test]
    fn to_snapshot_does_not_carry_the_target() {
        let mut map = GameMap::new();
        map.insert_tile(Position::new(1, 1, 7), crate::entities::map::MapTile::new());
        let victim = map
            .insert_agent(
                Agent::from_player(make_snapshot(2)),
                &Position::new(1, 1, 7),
            )
            .unwrap();

        let mut agent = Agent::from_player(make_snapshot(1));
        agent.set_target(Some(victim));

        let snapshot = agent.to_snapshot(Position::new(1, 1, 7)).unwrap();
        let restored = Agent::from_player(snapshot);

        assert!(restored.target().is_none());
    }
}
