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
        to_position: Position,
    },
    TileChanged {
        position: Position,
    },
    MoveDenied {
        agent_key: AgentKey,
        message: String,
    },
    MoveAck {
        agent_key: AgentKey,
    },
    OpenContainer {
        agent_key: AgentKey,
        item: ItemRef,
    },
    UseItemAck {
        agent_key: AgentKey,
        success: bool,
    },
    UpdateContainer {
        item: ItemRef,
    },
    PlayerWalkDenied {
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
    },
    AgentTeleport {
        agent_key: AgentKey,
        position: Position,
    },
    PlayerDespawned {
        agent_key: AgentKey,
        snapshot: Option<Arc<PlayerSnapshot>>,
    },
}
