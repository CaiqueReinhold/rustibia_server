use std::sync::Arc;

use crate::entities::{
    agent::{AgentKey, Facing},
    combat::CombatDamage,
    items::ItemRef,
    player::InventorySlot,
    position::{Direction, Position},
    skills::SkillType,
};
use crate::persistence::player::PlayerSnapshot;

#[derive(Clone, Debug)]
pub enum BroadcastMessage {
    PlayerSpawned {
        agent_key: AgentKey,
        position: Position,
    },
    AgentMoved {
        agent_key: AgentKey,
        direction: Direction,
        from_position: Position,
        to_position: Position,
    },
    TileChanged {
        position: Position,
    },
    MoveItemDenied {
        agent_key: AgentKey,
        message: String,
    },
    OpenContainer {
        agent_key: AgentKey,
        item: ItemRef,
    },
    UseItemDenied {
        agent_key: AgentKey,
        message: String,
    },
    ContainerUpdated {
        item: ItemRef,
    },
    AgentWalkDenied {
        agent_key: AgentKey,
    },
    UpdateInventorySlot {
        agent_key: AgentKey,
        slot: InventorySlot,
    },
    AgentChangedDirection {
        agent_key: AgentKey,
        facing: Facing,
        position: Position,
    },
    AgentTeleported {
        agent_key: AgentKey,
        from_position: Position,
        to_position: Position,
    },
    AgentDespawned {
        agent_key: AgentKey,
        position: Position,
        snapshot: Option<Arc<PlayerSnapshot>>,
    },
    LogoutDenied {
        agent_key: AgentKey,
    },
    AgentSaid {
        agent_key: AgentKey,
        message: String,
    },
    AgentLostTarget {
        agent_key: AgentKey,
        seq: u32,
    },
    DamageTaken {
        agent_key: AgentKey,
        position: Position,
        damage: CombatDamage,
    },
    MissileLaunched {
        from: Position,
        to: Position,
        sprite_id: u16,
    },
    SkillProgressUpdated {
        agent_key: AgentKey,
        skill_type: SkillType,
        amount: u64,
    },
    SkillUpgraded {
        agent_key: AgentKey,
        skill_type: SkillType,
        gained: u16,
    },
    PlayerManaUpdated {
        agent_key: AgentKey,
    },
    AgentLifeUpdated {
        agent_key: AgentKey,
    },
    PotionDrunk {
        target: AgentKey,
        position: Position,
    },
}
