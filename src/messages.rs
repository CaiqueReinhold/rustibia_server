use thiserror::Error;
use tokio_util::{
    bytes::{Buf, BufMut, BytesMut},
    codec::{Decoder, Encoder},
};

use crate::{
    constants::{MAX_VISIBLE_ITEMS, VIEWPORT_SIZE},
    entities::{
        agent::{AgentId, Facing, OutfitColors, OutfitId, Pool},
        items::{ContainerId, ItemId},
        player::InventorySlot,
        position::{Direction, Position},
    },
};

pub type ItemStack = [Option<(ItemId, u8)>; MAX_VISIBLE_ITEMS];

// client
const MSG_PING: u8 = 0;
const MSG_LOGIN: u8 = 1;
const MSG_MOVE_PLAYER: u8 = 2;
const MSG_GET_PLAYER_POS: u8 = 3;
const MSG_MOVE_ITEM: u8 = 4;
const MSG_USE_ITEM: u8 = 5;
const MSG_CLOSE_CONTAINER: u8 = 6;
const MSG_OPEN_PARENT_CONTAINER: u8 = 7;
const MSG_CHANGE_DIRECTION: u8 = 8;
const MSG_LOGOUT: u8 = 9;

#[derive(Clone, Debug)]
pub enum ClientMessage {
    Ping,
    Login {
        character_id: u32,
        auth_token: String,
    },
    MovePlayer {
        direction: Direction,
    },
    GetPlayerPosition,
    MoveItem {
        from: Position,
        item_id: ItemId,
        amount: u8,
        stack_index: u16,
        to: Position,
    },
    UseItem {
        position: Position,
        item_id: ItemId,
        stack_index: u16,
    },
    CloseContainer {
        container_id: ContainerId,
    },
    OpenParentContainer {
        container_id: ContainerId,
    },
    ChangeDirection {
        direction: Facing,
    },
    Logout,
}

// server
const MSG_PONG: u8 = 0;
const MSG_LOGIN_ERROR: u8 = 1;
const MSG_DESCRIBE_MAP: u8 = 2;
const MSG_TILE_CHANGED: u8 = 3;
const MSG_PLAYER_WALK_ACK: u8 = 4;
const MSG_PLAYER_POS: u8 = 5;
const MSG_DESCRIBE_PLAYER: u8 = 6;
const MSG_MOVE_ITEM_ACK: u8 = 7;
const MSG_MOVE_ITEM_DENIED: u8 = 8;
const MSG_TEXT_MESSAGE: u8 = 9;
const MSG_USE_ITEM_ACK: u8 = 10;
const MSG_OPEN_CONTAINER: u8 = 11;
const MSG_UPDATE_CONTAINER: u8 = 12;
const MSG_CONTAINER_CLOSED: u8 = 13;
const MSG_PLAYER_WALK_DENIED: u8 = 14;
const MSG_INVETORY_SLOT_UPDATED: u8 = 15;
const MSG_PLAYER_CAPACITY_UPDATED: u8 = 16;
const MSG_AGENT_DIRECTION_CHANGED: u8 = 17;
const MSG_REMOVE_AGENT: u8 = 18;
const MSG_MOVE_AGENT: u8 = 19;
const MSG_SPAWN_AGENT: u8 = 20;
const MSG_TELEPORT_AGENT: u8 = 21;

#[derive(Clone, Debug)]
pub enum TextMessageType {
    ActionDenied,
    LogoutDenied,
}

#[derive(Clone, Debug)]
pub enum ServerMessage {
    Pong,
    LoginError,
    DescribePlayer {
        agent_id: AgentId,
        position: Position,
        facing: Facing,
        name: String,
        level: u16,
        life: Pool,
        mana: Pool,
        outfit: (OutfitId, OutfitColors),
        speed: u16,
        capacity: u32,
        inventory_head: Option<ItemId>,
        inventory_amulet: Option<ItemId>,
        inventory_backpack: Option<ItemId>,
        inventory_chest: Option<ItemId>,
        inventory_right_hand: Option<ItemId>,
        inventory_left_hand: Option<ItemId>,
        inventory_legs: Option<ItemId>,
        inventory_feet: Option<ItemId>,
        inventory_ring: Option<ItemId>,
        inventory_trinket: Option<ItemId>,
    },
    DescribeMap {
        tiles: Box<[ItemStack; VIEWPORT_SIZE]>,
        center: Position,
        floor: u8,
    },
    TileChanged {
        position: Position,
        items: Box<ItemStack>,
    },
    PlayerWalkAck {
        position: Position,
        tiles: Vec<(u8, Box<[ItemStack]>)>, // (floor, tiles)
    },
    PlayerPosition {
        position: Position,
    },
    MoveItemAck,
    MoveItemDenied,
    TextMessage {
        text: String,
        message_type: TextMessageType,
    },
    UseItemAck,
    OpenContainer {
        container_id: ContainerId,
        capacity: u8,
        has_parent: bool,
        title: String,
        items: Box<[Option<(ItemId, u8)>]>,
    },
    UpdateContainer {
        container_id: ContainerId,
        items: Box<[Option<(ItemId, u8)>]>,
    },
    ContainerClosed {
        container_id: ContainerId,
    },
    PlayerWalkDenied,
    IventorySlotUpdated {
        slot: InventorySlot,
        item_id: Option<ItemId>,
    },
    PlayerCapacityUpdated {
        cap: u32,
    },
    AgentChangedDirection {
        agent_id: AgentId,
        facing: Facing,
    },
    RemoveAgent {
        agent_id: AgentId,
    },
    MoveAgent {
        agent_id: AgentId,
        direction: Direction,
        from: Position,
    },
    SpawnAgent {
        agent_id: AgentId,
        outfit: (OutfitId, OutfitColors),
        position: Position,
        facing: Facing,
        name: String,
        life: Pool,
        speed: u16,
    },
    TeleportAgent {
        agent_id: AgentId,
        position: Position,
    },
}

#[derive(Error, Debug)]
pub enum MessageDecodeError {
    #[error("Read error")]
    ReadError(#[from] std::io::Error),
    #[error("Wrong sequence")]
    WrongSequence,
}

pub struct GameMessageCodec {}

impl Decoder for GameMessageCodec {
    type Item = ClientMessage;
    type Error = MessageDecodeError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if buf.len() < 2 {
            return Ok(None);
        }

        let payload_len = u16::from_le_bytes([buf[0], buf[1]]) as usize;

        if buf.len() < 2 + payload_len {
            return Ok(None);
        }

        buf.advance(2);

        match buf.get_u8() {
            MSG_PING => Ok(Some(ClientMessage::Ping)),
            MSG_LOGIN => {
                let character_id = buf.get_u32_le();
                let token_len = payload_len - 1 - 4; // subtract msg type byte and character_id
                let auth_token = String::from_utf8(buf.split_to(token_len).to_vec())
                    .map_err(|_| MessageDecodeError::WrongSequence)?;
                Ok(Some(ClientMessage::Login {
                    character_id,
                    auth_token,
                }))
            }
            MSG_MOVE_PLAYER => {
                let direction = decode_direction(buf.get_u8())?;
                Ok(Some(ClientMessage::MovePlayer { direction }))
            }
            MSG_GET_PLAYER_POS => Ok(Some(ClientMessage::GetPlayerPosition)),
            MSG_MOVE_ITEM => {
                let from = decode_position(buf);
                let item_id = buf.get_u16_le();
                let amount = buf.get_u8();
                let stack_index = buf.get_u16_le();
                let to = decode_position(buf);
                Ok(Some(ClientMessage::MoveItem {
                    from,
                    item_id,
                    amount,
                    stack_index,
                    to,
                }))
            }
            MSG_USE_ITEM => Ok(Some(ClientMessage::UseItem {
                position: decode_position(buf),
                item_id: buf.get_u16_le(),
                stack_index: buf.get_u16_le(),
            })),
            MSG_CLOSE_CONTAINER => Ok(Some(ClientMessage::CloseContainer {
                container_id: buf.get_u16_le(),
            })),
            MSG_OPEN_PARENT_CONTAINER => Ok(Some(ClientMessage::OpenParentContainer {
                container_id: buf.get_u16_le(),
            })),
            MSG_CHANGE_DIRECTION => Ok(Some(ClientMessage::ChangeDirection {
                direction: decode_facing(buf.get_u8())?,
            })),
            MSG_LOGOUT => Ok(Some(ClientMessage::Logout)),
            _ => Err(MessageDecodeError::WrongSequence),
        }
    }
}

fn decode_position(buf: &mut BytesMut) -> Position {
    Position {
        x: buf.get_u16_le(),
        y: buf.get_u16_le(),
        z: buf.get_u8(),
    }
}

fn decode_direction(b: u8) -> Result<Direction, MessageDecodeError> {
    match b {
        0x00 => Ok(Direction::North),
        0x01 => Ok(Direction::East),
        0x02 => Ok(Direction::West),
        0x03 => Ok(Direction::South),
        0x04 => Ok(Direction::NorthEast),
        0x05 => Ok(Direction::NorthWest),
        0x06 => Ok(Direction::SouthEast),
        0x07 => Ok(Direction::SouthWest),
        _ => Err(MessageDecodeError::WrongSequence),
    }
}

fn decode_facing(b: u8) -> Result<Facing, MessageDecodeError> {
    match b {
        1 => Ok(Facing::North),
        2 => Ok(Facing::East),
        3 => Ok(Facing::South),
        4 => Ok(Facing::West),
        _ => Err(MessageDecodeError::WrongSequence),
    }
}

#[derive(Error, Debug)]
pub enum MessageEncodeError {
    #[error("Read error")]
    ReadError(#[from] std::io::Error),
}

impl Encoder<ServerMessage> for GameMessageCodec {
    type Error = MessageEncodeError;

    fn encode(&mut self, item: ServerMessage, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let len_offset = dst.len();
        dst.put_u16_le(0); // placeholder for payload length

        match item {
            ServerMessage::Pong => dst.put_u8(MSG_PONG),
            ServerMessage::LoginError => dst.put_u8(MSG_LOGIN_ERROR),
            ServerMessage::DescribePlayer {
                agent_id,
                position,
                facing,
                name,
                level,
                life,
                mana,
                outfit,
                speed,
                capacity,
                inventory_head,
                inventory_amulet,
                inventory_backpack,
                inventory_chest,
                inventory_right_hand,
                inventory_left_hand,
                inventory_legs,
                inventory_feet,
                inventory_ring,
                inventory_trinket,
            } => {
                dst.put_u8(MSG_DESCRIBE_PLAYER);
                dst.put_u16_le(agent_id);
                encode_position(position, dst);
                encode_facing(facing, dst);
                let name_bytes = name.as_bytes();
                dst.put_u16_le(name_bytes.len() as u16);
                dst.put_slice(name_bytes);
                dst.put_u16_le(level);
                dst.put_u32_le(life.current);
                dst.put_u32_le(life.maximum);
                dst.put_u32_le(mana.current);
                dst.put_u32_le(mana.maximum);
                dst.put_u16_le(outfit.0);
                dst.put_u8(outfit.1 .0);
                dst.put_u8(outfit.1 .1);
                dst.put_u8(outfit.1 .2);
                dst.put_u8(outfit.1 .3);
                dst.put_u16_le(speed);
                dst.put_u32_le(capacity);
                encode_optional_item(inventory_head, dst);
                encode_optional_item(inventory_amulet, dst);
                encode_optional_item(inventory_backpack, dst);
                encode_optional_item(inventory_chest, dst);
                encode_optional_item(inventory_right_hand, dst);
                encode_optional_item(inventory_left_hand, dst);
                encode_optional_item(inventory_legs, dst);
                encode_optional_item(inventory_feet, dst);
                encode_optional_item(inventory_ring, dst);
                encode_optional_item(inventory_trinket, dst);
            }
            ServerMessage::DescribeMap {
                tiles,
                center,
                floor,
            } => {
                dst.put_u8(MSG_DESCRIBE_MAP);
                encode_position(center, dst);
                dst.put_u8(floor);
                for tile in tiles.iter() {
                    encode_tile(tile.as_ref(), dst);
                }
            }
            ServerMessage::TileChanged { position, items } => {
                dst.put_u8(MSG_TILE_CHANGED);
                encode_position(position, dst);
                encode_tile(items.as_ref(), dst);
            }
            ServerMessage::PlayerWalkAck { position, tiles } => {
                dst.put_u8(MSG_PLAYER_WALK_ACK);
                encode_position(position, dst);
                for (floor, tiles) in tiles.iter() {
                    dst.put_u8(*floor);
                    dst.put_u8(tiles.len() as u8);
                    for tile in tiles.iter() {
                        encode_tile(tile, dst);
                    }
                }
                dst.put_u8(0xFF);
            }
            ServerMessage::PlayerPosition { position } => {
                dst.put_u8(MSG_PLAYER_POS);
                encode_position(position, dst);
            }
            ServerMessage::MoveItemAck => {
                dst.put_u8(MSG_MOVE_ITEM_ACK);
            }
            ServerMessage::MoveItemDenied => {
                dst.put_u8(MSG_MOVE_ITEM_DENIED);
            }
            ServerMessage::TextMessage { text, message_type } => {
                dst.put_u8(MSG_TEXT_MESSAGE);
                let text_bytes = text.as_bytes();
                dst.put_u16_le(text_bytes.len() as u16);
                dst.put_slice(text_bytes);
                dst.put_u8(encode_text_message_type(message_type));
            }
            ServerMessage::UseItemAck => dst.put_u8(MSG_USE_ITEM_ACK),
            ServerMessage::OpenContainer {
                container_id,
                capacity,
                has_parent,
                title,
                items,
            } => {
                dst.put_u8(MSG_OPEN_CONTAINER);
                dst.put_u16_le(container_id);
                dst.put_u8(capacity);
                dst.put_u8(if has_parent { 1 } else { 0 });
                let title_bytes = title.as_bytes();
                dst.put_u8(title_bytes.len() as u8);
                dst.put_slice(title_bytes);
                encode_tile(&items, dst);
            }
            ServerMessage::UpdateContainer {
                container_id,
                items,
            } => {
                dst.put_u8(MSG_UPDATE_CONTAINER);
                dst.put_u16_le(container_id);
                encode_tile(&items, dst);
            }
            ServerMessage::ContainerClosed { container_id } => {
                dst.put_u8(MSG_CONTAINER_CLOSED);
                dst.put_u16_le(container_id);
            }
            ServerMessage::PlayerWalkDenied => dst.put_u8(MSG_PLAYER_WALK_DENIED),
            ServerMessage::IventorySlotUpdated { slot, item_id } => {
                dst.put_u8(MSG_INVETORY_SLOT_UPDATED);
                dst.put_u8(slot.as_id() as u8);
                encode_optional_item(item_id, dst);
            }
            ServerMessage::PlayerCapacityUpdated { cap } => {
                dst.put_u8(MSG_PLAYER_CAPACITY_UPDATED);
                dst.put_u32_le(cap);
            }
            ServerMessage::AgentChangedDirection { agent_id, facing } => {
                dst.put_u8(MSG_AGENT_DIRECTION_CHANGED);
                dst.put_u16_le(agent_id);
                encode_facing(facing, dst);
            }
            ServerMessage::RemoveAgent { agent_id } => {
                dst.put_u8(MSG_REMOVE_AGENT);
                dst.put_u16_le(agent_id);
            }
            ServerMessage::MoveAgent {
                agent_id,
                direction,
                from,
            } => {
                dst.put_u8(MSG_MOVE_AGENT);
                dst.put_u16_le(agent_id);
                encode_direction(&direction, dst);
                encode_position(from, dst);
            }
            ServerMessage::SpawnAgent {
                agent_id,
                outfit,
                position,
                facing,
                name,
                life,
                speed,
            } => {
                dst.put_u8(MSG_SPAWN_AGENT);
                dst.put_u16_le(agent_id);
                encode_position(position, dst);
                encode_facing(facing, dst);
                let name_bytes = name.as_bytes();
                dst.put_u16_le(name_bytes.len() as u16);
                dst.put_slice(name_bytes);
                dst.put_u32_le(life.current);
                dst.put_u32_le(life.maximum);
                dst.put_u16_le(outfit.0);
                dst.put_u8(outfit.1 .0);
                dst.put_u8(outfit.1 .1);
                dst.put_u8(outfit.1 .2);
                dst.put_u8(outfit.1 .3);
                dst.put_u16_le(speed);
            }
            ServerMessage::TeleportAgent { agent_id, position } => {
                dst.put_u8(MSG_TELEPORT_AGENT);
                dst.put_u16_le(agent_id);
                encode_position(position, dst);
            }
        }

        let payload_len = (dst.len() - len_offset - 2) as u16;
        dst[len_offset..len_offset + 2].copy_from_slice(&payload_len.to_le_bytes());

        Ok(())
    }
}

fn encode_position(pos: Position, dst: &mut BytesMut) {
    dst.put_u16_le(pos.x);
    dst.put_u16_le(pos.y);
    dst.put_u8(pos.z);
}

fn encode_facing(facing: Facing, dst: &mut BytesMut) {
    match facing {
        Facing::North => dst.put_u8(1),
        Facing::East => dst.put_u8(2),
        Facing::South => dst.put_u8(3),
        Facing::West => dst.put_u8(4),
    }
}

fn encode_direction(d: &Direction, dst: &mut BytesMut) {
    let value = match d {
        Direction::North => 0x00,
        Direction::East => 0x01,
        Direction::West => 0x02,
        Direction::South => 0x03,
        Direction::NorthEast => 0x04,
        Direction::NorthWest => 0x05,
        Direction::SouthEast => 0x06,
        Direction::SouthWest => 0x07,
    };
    dst.put_u8(value);
}

fn encode_tile(items: &[Option<(ItemId, u8)>], dst: &mut BytesMut) {
    for item in items {
        match item {
            Some((id, amount)) => {
                dst.put_u16_le(*id);
                dst.put_u8(*amount);
            }
            None => break,
        }
    }
    dst.put_u16_le(0xFFFF);
}

fn encode_text_message_type(text_type: TextMessageType) -> u8 {
    match text_type {
        TextMessageType::ActionDenied => 0x01,
        TextMessageType::LogoutDenied => 0x02,
    }
}

fn encode_optional_item(item_id: Option<ItemId>, dst: &mut BytesMut) {
    if let Some(item_id) = item_id {
        dst.put_u16_le(item_id);
    } else {
        dst.put_u16_le(0xFFFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::bytes::BytesMut;
    use tokio_util::codec::Decoder;

    #[test]
    fn decode_logout_message() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        // 2-byte LE length prefix (1 byte payload) + 1 byte type
        buf.extend_from_slice(&[1u8, 0u8, MSG_LOGOUT]);
        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert!(matches!(msg, ClientMessage::Logout));
        assert!(buf.is_empty());
    }
}
