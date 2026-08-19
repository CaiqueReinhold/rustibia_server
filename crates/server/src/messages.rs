use thiserror::Error;
use tokio_util::{
    bytes::{Buf, BufMut, BytesMut},
    codec::{Decoder, Encoder},
};

use crate::{
    constants::{MAX_VISIBLE_ITEMS, VIEWPORT_SIZE},
    entities::{
        agent::{AgentId, Facing, OutfitColors, OutfitId, Pool},
        chat::{ChannelId, ChatMessageType},
        items::{ContainerId, ItemId},
        player::InventorySlot,
        position::{Direction, Position},
    },
};

pub type ItemStack = [Option<(ItemId, u8)>; MAX_VISIBLE_ITEMS];

// client
const CLI_PING: u8 = 0;
const CLI_LOGIN: u8 = 1;
const CLI_MOVE_PLAYER: u8 = 2;
const CLI_GET_PLAYER_POS: u8 = 3;
const CLI_MOVE_ITEM: u8 = 4;
const CLI_USE_ITEM: u8 = 5;
const CLI_CLOSE_CONTAINER: u8 = 6;
const CLI_OPEN_PARENT_CONTAINER: u8 = 7;
const CLI_CHANGE_DIRECTION: u8 = 8;
const CLI_LOGOUT: u8 = 9;
const CLI_USE_ITEM_WITH: u8 = 10;
const CLI_LOOK: u8 = 11;
const CLI_SAY: u8 = 12;
const CLI_REQUEST_CHANNELS: u8 = 13;
const CLI_OPEN_CHANNEL: u8 = 14;
const CLI_CLOSE_CHANNEL: u8 = 15;
const CLI_OPEN_PM_CHAT: u8 = 16;
const CLI_SET_TARGET: u8 = 17;

#[derive(Clone, Debug)]
pub enum ClientMessage {
    Ping,
    Login {
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
        stack_index: u8,
        to: Position,
    },
    UseItem {
        position: Position,
        item_id: ItemId,
        stack_index: u8,
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
    UseItemWith {
        source: Position,
        source_item_id: ItemId,
        source_index: u8,
        target: Position,
        target_item_id: ItemId,
        target_index: u8,
    },
    Look {
        position: Position,
    },
    Say {
        message: String,
        message_type: ChatMessageType,
        target: u16,
    },
    RequestChannels,
    OpenChannel {
        channel: ChannelId,
    },
    CloseChannel {
        channel: ChannelId,
    },
    OpenPmChat {
        name: String,
    },
    SetTarget {
        agent_id: Option<AgentId>,
    },
}

// server
const SRV_PONG: u8 = 0;
const SRV_LOGIN_ERROR: u8 = 1;
const SRV_DESCRIBE_MAP: u8 = 2;
const SRV_TILE_CHANGED: u8 = 3;
const SRV_PLAYER_WALK_ACK: u8 = 4;
const SRV_PLAYER_POS: u8 = 5;
const SRV_DESCRIBE_PLAYER: u8 = 6;
const SRV_TEXT_MESSAGE: u8 = 7;
const SRV_OPEN_CONTAINER: u8 = 8;
const SRV_UPDATE_CONTAINER: u8 = 9;
const SRV_CONTAINER_CLOSED: u8 = 10;
const SRV_PLAYER_WALK_DENIED: u8 = 11;
const SRV_INVETORY_SLOT_UPDATED: u8 = 12;
const SRV_PLAYER_CAPACITY_UPDATED: u8 = 13;
const SRV_AGENT_DIRECTION_CHANGED: u8 = 14;
const SRV_REMOVE_AGENT: u8 = 15;
const SRV_MOVE_AGENT: u8 = 16;
const SRV_SPAWN_AGENT: u8 = 17;
const SRV_TELEPORT_AGENT: u8 = 18;
const SRV_CHAT_MESSAGE: u8 = 19;
const SRV_CHANNEL_LIST: u8 = 20;
const SRV_INTRODUCE_PLAYER: u8 = 21;
const SRV_FLOATING_TEXT: u8 = 22;
const SRV_TARGET_CHANGED: u8 = 23;

#[derive(Clone, Debug)]
pub enum TextMessageType {
    ActionDenied,
    Look,
}

/// Which kind of world-anchored text the client should draw. A presentation
/// concept only — nothing in `game/` or `entities/` models it, which is why it
/// lives here beside `TextMessageType` rather than under `entities/`.
///
/// Unused by `main` for now: nothing constructs `ServerMessage::FloatingText` yet
/// (combat doesn't exist), so both variants are dead code until that producer lands
/// in a later task. `#[allow(dead_code)]` mirrors the convention already used for
/// `SqlLoginRepository` in `persistence/login.rs`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub enum FloatingTextType {
    HitPoints,
    PlayerMessage,
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
    TextMessage {
        text: String,
        message_type: TextMessageType,
    },
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
    ChatMessage {
        author: AgentId,
        message_type: ChatMessageType,
        channel: u16,
        message: String,
    },
    ChannelList {
        channels: Vec<(ChannelId, String)>,
    },
    IntroducePlayer {
        local_id: AgentId,
        name: String,
    },
    /// Unused by `main` for now: no producer exists until combat lands in a later
    /// task. See the note on `FloatingTextType`.
    #[allow(dead_code)]
    FloatingText {
        text: String,
        position: Position,
        text_type: FloatingTextType,
        color: Option<(u8, u8, u8)>,
    },
    TargetChanged {
        agent_id: Option<AgentId>,
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

        // A payload always carries at least its message-type byte, so a declared length
        // of zero is malformed rather than merely incomplete. Rejecting it here is what
        // keeps the two lines below total: `get_u8` would panic on the empty buffer a
        // two-byte frame leaves behind, and any arm computing `payload_len - 1` would
        // underflow — panicking in debug, or wrapping to `usize::MAX` in release and
        // panicking inside `split_to`'s bounds check instead. Both are reachable from a
        // hand-crafted frame, so this is a guard against hostile input, not a tidy-up.
        if payload_len == 0 {
            return Err(MessageDecodeError::WrongSequence);
        }

        buf.advance(2);

        match buf.get_u8() {
            CLI_PING => Ok(Some(ClientMessage::Ping)),
            CLI_LOGIN => {
                // Cannot underflow: `payload_len` is at least 1, checked above.
                let token_len = payload_len - 1; // subtract the message type byte
                let auth_token = String::from_utf8(buf.split_to(token_len).to_vec())
                    .map_err(|_| MessageDecodeError::WrongSequence)?;
                Ok(Some(ClientMessage::Login { auth_token }))
            }
            CLI_MOVE_PLAYER => {
                let direction = decode_direction(buf.get_u8())?;
                Ok(Some(ClientMessage::MovePlayer { direction }))
            }
            CLI_GET_PLAYER_POS => Ok(Some(ClientMessage::GetPlayerPosition)),
            CLI_MOVE_ITEM => {
                let from = decode_position(buf);
                let item_id = buf.get_u16_le();
                let amount = buf.get_u8();
                let stack_index = buf.get_u8();
                let to = decode_position(buf);
                Ok(Some(ClientMessage::MoveItem {
                    from,
                    item_id,
                    amount,
                    stack_index,
                    to,
                }))
            }
            CLI_USE_ITEM => Ok(Some(ClientMessage::UseItem {
                position: decode_position(buf),
                item_id: buf.get_u16_le(),
                stack_index: buf.get_u8(),
            })),
            CLI_CLOSE_CONTAINER => Ok(Some(ClientMessage::CloseContainer {
                container_id: buf.get_u16_le(),
            })),
            CLI_OPEN_PARENT_CONTAINER => Ok(Some(ClientMessage::OpenParentContainer {
                container_id: buf.get_u16_le(),
            })),
            CLI_CHANGE_DIRECTION => Ok(Some(ClientMessage::ChangeDirection {
                direction: decode_facing(buf.get_u8())?,
            })),
            CLI_LOGOUT => Ok(Some(ClientMessage::Logout)),
            CLI_USE_ITEM_WITH => Ok(Some(ClientMessage::UseItemWith {
                source: decode_position(buf),
                source_item_id: buf.get_u16_le(),
                source_index: buf.get_u8(),
                target: decode_position(buf),
                target_item_id: buf.get_u16_le(),
                target_index: buf.get_u8(),
            })),
            CLI_LOOK => Ok(Some(ClientMessage::Look {
                position: decode_position(buf),
            })),
            CLI_SAY => {
                // opcode + message type + 2 target bytes. Guard before subtracting: a
                // hand-crafted short frame would otherwise wrap to `usize::MAX` in
                // release and panic inside `split_to`.
                if payload_len < 4 {
                    return Err(MessageDecodeError::WrongSequence);
                }
                let message_type = decode_chat_message_type(buf.get_u8())?;
                let target = buf.get_u16_le();
                let message = String::from_utf8(buf.split_to(payload_len - 4).to_vec())
                    .map_err(|_| MessageDecodeError::WrongSequence)?;
                Ok(Some(ClientMessage::Say {
                    message,
                    message_type,
                    target,
                }))
            }
            CLI_REQUEST_CHANNELS => Ok(Some(ClientMessage::RequestChannels)),
            CLI_OPEN_CHANNEL => Ok(Some(ClientMessage::OpenChannel {
                channel: buf.get_u16_le(),
            })),
            CLI_CLOSE_CHANNEL => Ok(Some(ClientMessage::CloseChannel {
                channel: buf.get_u16_le(),
            })),
            CLI_OPEN_PM_CHAT => {
                // Cannot underflow: `payload_len` is at least 1, checked above.
                let name = String::from_utf8(buf.split_to(payload_len - 1).to_vec())
                    .map_err(|_| MessageDecodeError::WrongSequence)?;
                Ok(Some(ClientMessage::OpenPmChat { name }))
            }
            CLI_SET_TARGET => Ok(Some(ClientMessage::SetTarget {
                agent_id: decode_optional_agent(buf.get_u16_le()),
            })),
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

fn decode_chat_message_type(b: u8) -> Result<ChatMessageType, MessageDecodeError> {
    match b {
        0x01 => Ok(ChatMessageType::Local),
        0x02 => Ok(ChatMessageType::Private),
        0x03 => Ok(ChatMessageType::Channel),
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
            ServerMessage::Pong => dst.put_u8(SRV_PONG),
            ServerMessage::LoginError => dst.put_u8(SRV_LOGIN_ERROR),
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
                dst.put_u8(SRV_DESCRIBE_PLAYER);
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
                dst.put_u8(outfit.1.0);
                dst.put_u8(outfit.1.1);
                dst.put_u8(outfit.1.2);
                dst.put_u8(outfit.1.3);
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
                dst.put_u8(SRV_DESCRIBE_MAP);
                encode_position(center, dst);
                dst.put_u8(floor);
                for tile in tiles.iter() {
                    encode_tile(tile.as_ref(), dst);
                }
            }
            ServerMessage::TileChanged { position, items } => {
                dst.put_u8(SRV_TILE_CHANGED);
                encode_position(position, dst);
                encode_tile(items.as_ref(), dst);
            }
            ServerMessage::PlayerWalkAck { position, tiles } => {
                dst.put_u8(SRV_PLAYER_WALK_ACK);
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
                dst.put_u8(SRV_PLAYER_POS);
                encode_position(position, dst);
            }
            ServerMessage::TextMessage { text, message_type } => {
                dst.put_u8(SRV_TEXT_MESSAGE);
                let text_bytes = text.as_bytes();
                dst.put_u16_le(text_bytes.len() as u16);
                dst.put_slice(text_bytes);
                dst.put_u8(encode_text_message_type(message_type));
            }
            ServerMessage::OpenContainer {
                container_id,
                capacity,
                has_parent,
                title,
                items,
            } => {
                dst.put_u8(SRV_OPEN_CONTAINER);
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
                dst.put_u8(SRV_UPDATE_CONTAINER);
                dst.put_u16_le(container_id);
                encode_tile(&items, dst);
            }
            ServerMessage::ContainerClosed { container_id } => {
                dst.put_u8(SRV_CONTAINER_CLOSED);
                dst.put_u16_le(container_id);
            }
            ServerMessage::PlayerWalkDenied => dst.put_u8(SRV_PLAYER_WALK_DENIED),
            ServerMessage::IventorySlotUpdated { slot, item_id } => {
                dst.put_u8(SRV_INVETORY_SLOT_UPDATED);
                dst.put_u8(slot.as_id() as u8);
                encode_optional_item(item_id, dst);
            }
            ServerMessage::PlayerCapacityUpdated { cap } => {
                dst.put_u8(SRV_PLAYER_CAPACITY_UPDATED);
                dst.put_u32_le(cap);
            }
            ServerMessage::AgentChangedDirection { agent_id, facing } => {
                dst.put_u8(SRV_AGENT_DIRECTION_CHANGED);
                dst.put_u16_le(agent_id);
                encode_facing(facing, dst);
            }
            ServerMessage::RemoveAgent { agent_id } => {
                dst.put_u8(SRV_REMOVE_AGENT);
                dst.put_u16_le(agent_id);
            }
            ServerMessage::MoveAgent {
                agent_id,
                direction,
                from,
            } => {
                dst.put_u8(SRV_MOVE_AGENT);
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
                dst.put_u8(SRV_SPAWN_AGENT);
                dst.put_u16_le(agent_id);
                encode_position(position, dst);
                encode_facing(facing, dst);
                let name_bytes = name.as_bytes();
                dst.put_u16_le(name_bytes.len() as u16);
                dst.put_slice(name_bytes);
                dst.put_u32_le(life.current);
                dst.put_u32_le(life.maximum);
                dst.put_u16_le(outfit.0);
                dst.put_u8(outfit.1.0);
                dst.put_u8(outfit.1.1);
                dst.put_u8(outfit.1.2);
                dst.put_u8(outfit.1.3);
                dst.put_u16_le(speed);
            }
            ServerMessage::TeleportAgent { agent_id, position } => {
                dst.put_u8(SRV_TELEPORT_AGENT);
                dst.put_u16_le(agent_id);
                encode_position(position, dst);
            }
            ServerMessage::ChatMessage {
                author,
                message_type,
                channel,
                message,
            } => {
                dst.put_u8(SRV_CHAT_MESSAGE);
                dst.put_u16_le(author);
                dst.put_u8(encode_chat_message_type(message_type));
                dst.put_u16_le(channel);
                let message_bytes = message.as_bytes();
                dst.put_u16_le(message_bytes.len() as u16);
                dst.put_slice(message_bytes);
            }
            ServerMessage::ChannelList { channels } => {
                dst.put_u8(SRV_CHANNEL_LIST);
                dst.put_u16_le(channels.len() as u16);
                for (id, name) in channels.iter() {
                    dst.put_u16_le(*id);
                    let name_bytes = name.as_bytes();
                    dst.put_u16_le(name_bytes.len() as u16);
                    dst.put_slice(name_bytes);
                }
            }
            ServerMessage::IntroducePlayer { local_id, name } => {
                dst.put_u8(SRV_INTRODUCE_PLAYER);
                dst.put_u16_le(local_id);
                let name_bytes = name.as_bytes();
                dst.put_u16_le(name_bytes.len() as u16);
                dst.put_slice(name_bytes);
            }
            ServerMessage::FloatingText {
                text,
                position,
                text_type,
                color,
            } => {
                dst.put_u8(SRV_FLOATING_TEXT);
                let text_bytes = text.as_bytes();
                dst.put_u16_le(text_bytes.len() as u16);
                dst.put_slice(text_bytes);
                encode_position(position, dst);
                dst.put_u8(encode_floating_text_type(text_type));
                match color {
                    Some((r, g, b)) => {
                        dst.put_u8(0x01);
                        dst.put_u8(r);
                        dst.put_u8(g);
                        dst.put_u8(b);
                    }
                    None => dst.put_u8(0x00),
                }
            }
            ServerMessage::TargetChanged { agent_id } => {
                dst.put_u8(SRV_TARGET_CHANGED);
                encode_optional_agent(agent_id, dst);
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
        TextMessageType::Look => 0x02,
    }
}

fn encode_optional_item(item_id: Option<ItemId>, dst: &mut BytesMut) {
    if let Some(item_id) = item_id {
        dst.put_u16_le(item_id);
    } else {
        dst.put_u16_le(0xFFFF);
    }
}

/// `0xFFFF` is the same "absent" sentinel `encode_optional_item` uses. It cannot
/// collide with a real id: `LocalIdMap` allocates from 1 upward and recycles, so
/// reaching 65535 would need 65534 agents visible in one 15x11 viewport.
fn encode_optional_agent(agent_id: Option<AgentId>, dst: &mut BytesMut) {
    dst.put_u16_le(agent_id.unwrap_or(0xFFFF));
}

fn decode_optional_agent(raw: u16) -> Option<AgentId> {
    if raw == 0xFFFF { None } else { Some(raw) }
}

fn encode_chat_message_type(message_type: ChatMessageType) -> u8 {
    match message_type {
        ChatMessageType::Local => 0x01,
        ChatMessageType::Private => 0x02,
        ChatMessageType::Channel => 0x03,
    }
}

fn encode_floating_text_type(text_type: FloatingTextType) -> u8 {
    match text_type {
        FloatingTextType::HitPoints => 0x01,
        FloatingTextType::PlayerMessage => 0x02,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::bytes::BytesMut;
    use tokio_util::codec::Decoder;
    use tokio_util::codec::Encoder;

    #[test]
    fn decode_logout_message() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        // 2-byte LE length prefix (1 byte payload) + 1 byte type
        buf.extend_from_slice(&[1u8, 0u8, CLI_LOGOUT]);
        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert!(matches!(msg, ClientMessage::Logout));
        assert!(buf.is_empty());
    }

    /// The frame the client now sends: length, type byte, then the token and nothing
    /// else. The character is no longer the client's to name.
    #[test]
    fn decode_login_message() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        let token = b"a-token";
        let payload_len = (1 + token.len()) as u16;

        buf.extend_from_slice(&payload_len.to_le_bytes());
        buf.extend_from_slice(&[CLI_LOGIN]);
        buf.extend_from_slice(token);

        let msg = codec.decode(&mut buf).unwrap().unwrap();

        match msg {
            ClientMessage::Login { auth_token } => assert_eq!(auth_token, "a-token"),
            other => panic!("expected Login, got {other:?}"),
        }
        assert!(buf.is_empty(), "the frame must be fully consumed");
    }

    /// A frame in the old layout is not rejected structurally — `token_len` comes from
    /// the frame length, so the 4-byte id is simply eaten as part of the token. Whether
    /// that yields a junk token (refused later, at redemption) or a decode error
    /// depends on the id's bytes. Both are fine. What must not happen is a panic in
    /// `from_utf8` or on a short buffer.
    #[test]
    fn an_old_format_login_frame_fails_without_panicking() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        let token = b"a-token";
        let payload_len = (1 + 4 + token.len()) as u16;

        buf.extend_from_slice(&payload_len.to_le_bytes());
        buf.extend_from_slice(&[CLI_LOGIN]);
        buf.extend_from_slice(&7u32.to_le_bytes());
        buf.extend_from_slice(token);

        match codec.decode(&mut buf) {
            Ok(Some(ClientMessage::Login { auth_token })) => assert_ne!(
                auth_token, "a-token",
                "the id bytes must land inside the token, not be silently skipped"
            ),
            Err(MessageDecodeError::WrongSequence) => {}
            other => panic!("expected a junk token or WrongSequence, got {other:?}"),
        }
    }

    /// A two-byte frame declaring a zero-length payload. Before the length guard this
    /// panicked in `get_u8` on the empty buffer `advance(2)` left behind — reachable by
    /// anyone who can open a socket, and for every message type, not just login.
    #[test]
    fn a_zero_length_payload_is_rejected_and_does_not_panic() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&0u16.to_le_bytes());

        assert!(
            matches!(
                codec.decode(&mut buf),
                Err(MessageDecodeError::WrongSequence)
            ),
            "a zero-length payload must be a decode error, never a panic"
        );
    }

    /// The same malformed length, but with a type byte present so the old code reached
    /// `payload_len - 1`. That subtraction underflowed: a panic in debug, and in release
    /// a wrap to `usize::MAX` that panicked inside `split_to`'s bounds check instead.
    #[test]
    fn a_login_frame_claiming_zero_length_does_not_underflow() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&[CLI_LOGIN]);

        assert!(
            matches!(
                codec.decode(&mut buf),
                Err(MessageDecodeError::WrongSequence)
            ),
            "a zero-length login payload must be a decode error, never a panic"
        );
    }

    #[test]
    fn encode_introduce_player() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();

        codec
            .encode(
                ServerMessage::IntroducePlayer {
                    local_id: 3,
                    name: "Rizael".to_owned(),
                },
                &mut buf,
            )
            .unwrap();

        let payload_len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
        assert_eq!(
            payload_len,
            buf.len() - 2,
            "length prefix must cover the payload"
        );
        assert_eq!(buf[2], SRV_INTRODUCE_PLAYER);
        assert_eq!(u16::from_le_bytes([buf[3], buf[4]]), 3);
        assert_eq!(u16::from_le_bytes([buf[5], buf[6]]), 6);
        assert_eq!(&buf[7..], b"Rizael");
    }

    /// The colour is an `Option` behind a flag byte, so the two forms differ in
    /// length. Both are asserted because a decoder that always reads three colour
    /// bytes passes the `Some` case and corrupts the `None` case.
    #[test]
    fn encode_floating_text_with_a_colour() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();

        codec
            .encode(
                ServerMessage::FloatingText {
                    text: "-25".to_owned(),
                    position: Position::new(100, 200, 7),
                    text_type: FloatingTextType::HitPoints,
                    color: Some((255, 0, 64)),
                },
                &mut buf,
            )
            .unwrap();

        let payload_len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
        assert_eq!(
            payload_len,
            buf.len() - 2,
            "length prefix must cover the payload"
        );
        assert_eq!(buf[2], SRV_FLOATING_TEXT);
        assert_eq!(u16::from_le_bytes([buf[3], buf[4]]), 3, "text length");
        assert_eq!(&buf[5..8], b"-25");
        assert_eq!(u16::from_le_bytes([buf[8], buf[9]]), 100, "position x");
        assert_eq!(u16::from_le_bytes([buf[10], buf[11]]), 200, "position y");
        assert_eq!(buf[12], 7, "position z");
        assert_eq!(buf[13], 0x01, "HitPoints");
        assert_eq!(buf[14], 0x01, "colour present");
        assert_eq!(&buf[15..18], &[255, 0, 64], "rgb");
        assert_eq!(buf.len(), 18, "no trailing bytes");
    }

    /// The length prefix is a *byte* count. This codebase has been bitten by
    /// byte-vs-char confusion before — `actors/session.rs` pins `max_message_length`
    /// against `"é".repeat(..)` for the same reason — and every other test here uses
    /// ASCII, where the two are indistinguishable.
    #[test]
    fn encode_floating_text_measures_the_text_in_bytes() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        let text = "café"; // 4 chars, 5 bytes

        codec
            .encode(
                ServerMessage::FloatingText {
                    text: text.to_owned(),
                    position: Position::new(1, 2, 7),
                    text_type: FloatingTextType::PlayerMessage,
                    color: None,
                },
                &mut buf,
            )
            .unwrap();

        assert_eq!(text.chars().count(), 4, "fixture must be multi-byte");
        assert_eq!(
            u16::from_le_bytes([buf[3], buf[4]]),
            5,
            "the prefix counts bytes, not characters"
        );
        assert_eq!(&buf[5..10], text.as_bytes());
    }

    #[test]
    fn encode_floating_text_without_a_colour() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();

        codec
            .encode(
                ServerMessage::FloatingText {
                    text: "hi".to_owned(),
                    position: Position::new(1, 2, 7),
                    text_type: FloatingTextType::PlayerMessage,
                    color: None,
                },
                &mut buf,
            )
            .unwrap();

        assert_eq!(buf[2], SRV_FLOATING_TEXT);
        assert_eq!(u16::from_le_bytes([buf[3], buf[4]]), 2, "text length");
        assert_eq!(&buf[5..7], b"hi");
        assert_eq!(buf[12], 0x02, "PlayerMessage");
        assert_eq!(buf[13], 0x00, "colour absent");
        assert_eq!(
            buf.len(),
            14,
            "the None form is three bytes shorter than the Some form"
        );
    }

    #[test]
    fn encode_channel_list() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();

        codec
            .encode(
                ServerMessage::ChannelList {
                    channels: vec![(1, "World Chat".to_owned())],
                },
                &mut buf,
            )
            .unwrap();

        let payload_len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
        assert_eq!(payload_len, buf.len() - 2);
        assert_eq!(buf[2], SRV_CHANNEL_LIST);
        assert_eq!(u16::from_le_bytes([buf[3], buf[4]]), 1, "channel count");
        assert_eq!(u16::from_le_bytes([buf[5], buf[6]]), 1, "channel id");
        assert_eq!(u16::from_le_bytes([buf[7], buf[8]]), 10, "name length");
        assert_eq!(&buf[9..], b"World Chat");
    }

    /// `encode_channel_list` above only ever supplies one channel, so it pins the shape
    /// of a single `(id, name)` entry and the leading count but proves nothing about the
    /// loop: a `break` after the first iteration, a count field that disagrees with the
    /// number of entries actually written, or a wrong per-entry stride would all still
    /// pass it. This test walks a moving cursor across three entries with different-length
    /// names (so a fixed-stride bug can't hide) and a deliberately non-contiguous third id
    /// (so an implementation that emits a loop index instead of the real id fails too).
    #[test]
    fn encode_channel_list_writes_every_entry_in_order() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();

        let channels = vec![
            (1u16, "World Chat".to_owned()),
            (2u16, "Advertising".to_owned()),
            (7u16, "Help".to_owned()),
        ];

        codec
            .encode(
                ServerMessage::ChannelList {
                    channels: channels.clone(),
                },
                &mut buf,
            )
            .unwrap();

        let payload_len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
        assert_eq!(
            payload_len,
            buf.len() - 2,
            "length prefix must cover exactly the payload"
        );
        assert_eq!(buf[2], SRV_CHANNEL_LIST);

        fn read_u16(buf: &BytesMut, cursor: &mut usize) -> u16 {
            let value = u16::from_le_bytes([buf[*cursor], buf[*cursor + 1]]);
            *cursor += 2;
            value
        }

        let mut cursor = 3;
        let count = read_u16(&buf, &mut cursor);
        assert_eq!(count, channels.len() as u16, "channel count");

        for (expected_id, expected_name) in channels.iter() {
            let id = read_u16(&buf, &mut cursor);
            assert_eq!(id, *expected_id, "channel id for {expected_name}");

            let name_len = read_u16(&buf, &mut cursor) as usize;
            assert_eq!(
                name_len,
                expected_name.len(),
                "name length for id {expected_id}"
            );

            let name_bytes = &buf[cursor..cursor + name_len];
            assert_eq!(
                name_bytes,
                expected_name.as_bytes(),
                "name bytes for id {expected_id}"
            );
            cursor += name_len;
        }

        assert_eq!(
            cursor,
            buf.len(),
            "buffer must be fully consumed: nothing trailing after the last name"
        );
    }

    #[test]
    fn decode_local_say_message() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        let text = b"hello";
        let payload_len = (1 + 1 + 2 + text.len()) as u16;

        buf.extend_from_slice(&payload_len.to_le_bytes());
        buf.extend_from_slice(&[CLI_SAY, 0x01]);
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(text);

        match codec.decode(&mut buf).unwrap().unwrap() {
            ClientMessage::Say {
                message,
                message_type,
                target,
            } => {
                assert_eq!(message, "hello");
                assert!(matches!(message_type, ChatMessageType::Local));
                assert_eq!(target, 0);
            }
            other => panic!("expected Say, got {other:?}"),
        }
        assert!(buf.is_empty(), "the frame must be fully consumed");
    }

    #[test]
    fn decode_channel_say_carries_the_channel_in_target() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        let text = b"hi all";
        let payload_len = (1 + 1 + 2 + text.len()) as u16;

        buf.extend_from_slice(&payload_len.to_le_bytes());
        buf.extend_from_slice(&[CLI_SAY, 0x03]);
        buf.extend_from_slice(&7u16.to_le_bytes());
        buf.extend_from_slice(text);

        match codec.decode(&mut buf).unwrap().unwrap() {
            ClientMessage::Say {
                message_type,
                target,
                ..
            } => {
                assert!(matches!(message_type, ChatMessageType::Channel));
                assert_eq!(target, 7);
            }
            other => panic!("expected Say, got {other:?}"),
        }
    }

    /// A truncated Say frame must not underflow `payload_len - 4`.
    #[test]
    fn a_short_say_frame_is_rejected_without_panicking() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&[CLI_SAY, 0x01]);

        assert!(matches!(
            codec.decode(&mut buf),
            Err(MessageDecodeError::WrongSequence)
        ));
    }

    #[test]
    fn decode_unknown_chat_message_type_is_rejected() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        let payload_len = (1 + 1 + 2) as u16;

        buf.extend_from_slice(&payload_len.to_le_bytes());
        buf.extend_from_slice(&[CLI_SAY, 0x09]);
        buf.extend_from_slice(&0u16.to_le_bytes());

        assert!(matches!(
            codec.decode(&mut buf),
            Err(MessageDecodeError::WrongSequence)
        ));
    }

    #[test]
    fn decode_open_pm_chat_message() {
        let mut codec = GameMessageCodec {};
        let mut buf = BytesMut::new();
        let name = b"Rizael";
        let payload_len = (1 + name.len()) as u16;

        buf.extend_from_slice(&payload_len.to_le_bytes());
        buf.extend_from_slice(&[CLI_OPEN_PM_CHAT]);
        buf.extend_from_slice(name);

        match codec.decode(&mut buf).unwrap().unwrap() {
            ClientMessage::OpenPmChat { name } => assert_eq!(name, "Rizael"),
            other => panic!("expected OpenPmChat, got {other:?}"),
        }
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_channel_control_messages() {
        let mut codec = GameMessageCodec {};

        let mut buf = BytesMut::new();
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&[CLI_REQUEST_CHANNELS]);
        assert!(matches!(
            codec.decode(&mut buf).unwrap().unwrap(),
            ClientMessage::RequestChannels
        ));

        let mut buf = BytesMut::new();
        buf.extend_from_slice(&3u16.to_le_bytes());
        buf.extend_from_slice(&[CLI_OPEN_CHANNEL]);
        buf.extend_from_slice(&2u16.to_le_bytes());
        match codec.decode(&mut buf).unwrap().unwrap() {
            ClientMessage::OpenChannel { channel } => assert_eq!(channel, 2),
            other => panic!("expected OpenChannel, got {other:?}"),
        }

        let mut buf = BytesMut::new();
        buf.extend_from_slice(&3u16.to_le_bytes());
        buf.extend_from_slice(&[CLI_CLOSE_CHANNEL]);
        buf.extend_from_slice(&2u16.to_le_bytes());
        match codec.decode(&mut buf).unwrap().unwrap() {
            ClientMessage::CloseChannel { channel } => assert_eq!(channel, 2),
            other => panic!("expected CloseChannel, got {other:?}"),
        }
    }

    #[test]
    fn set_target_decodes_some_and_none() {
        let mut buf = BytesMut::new();
        // payload: opcode + u16 agent id
        buf.put_u16_le(3);
        buf.put_u8(CLI_SET_TARGET);
        buf.put_u16_le(7);
        let decoded = GameMessageCodec {}.decode(&mut buf).unwrap().unwrap();
        assert!(matches!(
            decoded,
            ClientMessage::SetTarget { agent_id: Some(7) }
        ));

        let mut buf = BytesMut::new();
        buf.put_u16_le(3);
        buf.put_u8(CLI_SET_TARGET);
        buf.put_u16_le(0xFFFF);
        let decoded = GameMessageCodec {}.decode(&mut buf).unwrap().unwrap();
        assert!(matches!(
            decoded,
            ClientMessage::SetTarget { agent_id: None }
        ));
    }

    #[test]
    fn target_changed_encodes_some_and_none() {
        let mut dst = BytesMut::new();
        GameMessageCodec {}
            .encode(ServerMessage::TargetChanged { agent_id: Some(9) }, &mut dst)
            .unwrap();
        // 2 bytes length prefix, then opcode, then the id
        assert_eq!(dst[2], SRV_TARGET_CHANGED);
        assert_eq!(u16::from_le_bytes([dst[3], dst[4]]), 9);

        let mut dst = BytesMut::new();
        GameMessageCodec {}
            .encode(ServerMessage::TargetChanged { agent_id: None }, &mut dst)
            .unwrap();
        assert_eq!(dst[2], SRV_TARGET_CHANGED);
        assert_eq!(u16::from_le_bytes([dst[3], dst[4]]), 0xFFFF);
    }
}
