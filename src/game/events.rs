use crate::entities::{
    agent::{AgentKey, Facing},
    items::ItemGuid,
    player::InventorySlot,
    position::{Direction, ItemPlacement, Position},
};

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
        guid: ItemGuid,
        placement: ItemPlacement,
    },
    UseItemAck {
        agent_key: AgentKey,
        success: bool,
    },
    UpdateContainer {
        guid: ItemGuid,
        placement: ItemPlacement,
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
    PlayerDespawned {
        agent_key: AgentKey,
    },
}
