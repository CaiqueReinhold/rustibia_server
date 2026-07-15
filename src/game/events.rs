use std::sync::Arc;

use crate::entities::{
    agent::{AgentKey, Facing},
    items::ItemRef,
    player::InventorySlot,
    position::{Direction, Position},
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
    UpdateContainer {
        item: ItemRef,
    },
    AgentWalkDenied {
        agent_key: AgentKey,
    },
    UpdateInventorySlot {
        agent_key: AgentKey,
        slot: InventorySlot,
    },
    UpdatePlayerCapacity {
        agent_key: AgentKey,
    },
    AgentChangedDirection {
        agent_key: AgentKey,
        facing: Facing,
        position: Position,
    },
    AgentTeleport {
        agent_key: AgentKey,
        from_position: Position,
        to_position: Position,
    },
    PlayerDespawned {
        agent_key: AgentKey,
        position: Position,
        snapshot: Option<Arc<PlayerSnapshot>>,
    },
    LogoutDenied {
        agent_key: AgentKey,
    },
}
