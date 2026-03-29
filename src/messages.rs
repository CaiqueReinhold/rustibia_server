use thiserror::Error;
use tokio_util::{
    bytes::{Buf, BufMut, BytesMut},
    codec::{Decoder, Encoder},
};

use crate::{
    constants::{MAX_VISIBLE_ITEMS, VIEWPORT_SIZE},
    entities::{
        items::{ContainerId, ItemId},
        player::{OutfitId, Pool},
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

#[derive(Clone, Debug)]
pub enum TextMessageType {
    ActionDenied,
}

#[derive(Clone, Debug)]
pub enum ServerMessage {
    Pong,
    LoginError,
    DescribePlayer {
        position: Position,
        name: String,
        level: u16,
        life: Pool,
        mana: Pool,
        outfit: OutfitId,
        speed: u16,
    },
    DescribeMap {
        tiles: Box<[ItemStack; VIEWPORT_SIZE]>,
    },
    TileChanged {
        position: Position,
        items: Box<ItemStack>,
    },
    PlayerWalkAck {
        position: Position,
        tiles: Box<[ItemStack]>,
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
        title: String,
        items: Box<[Option<(ItemId, u8)>]>,
    },
    UpdateContainer {
        container_id: ContainerId,
        items: Box<[Option<(ItemId, u8)>]>,
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
            _ => Err(MessageDecodeError::WrongSequence),
        }
    }
}

fn decode_position(buf: &mut BytesMut) -> Position {
    Position {
        x: buf.get_u32_le(),
        y: buf.get_u32_le(),
        z: buf.get_u32_le(),
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
                position,
                name,
                level,
                life,
                mana,
                outfit,
                speed,
            } => {
                dst.put_u8(MSG_DESCRIBE_PLAYER);
                encode_position(position, dst);
                let name_bytes = name.as_bytes();
                dst.put_u16_le(name_bytes.len() as u16);
                dst.put_slice(name_bytes);
                dst.put_u16_le(level);
                dst.put_u32_le(life.current);
                dst.put_u32_le(life.maximum);
                dst.put_u32_le(mana.current);
                dst.put_u32_le(mana.maximum);
                dst.put_u16_le(outfit);
                dst.put_u16_le(speed);
            }
            ServerMessage::DescribeMap { tiles } => {
                dst.put_u8(MSG_DESCRIBE_MAP);
                for tile in tiles.iter() {
                    encode_tile(tile.as_ref(), dst);
                }
            }
            ServerMessage::TileChanged { position, items } => {
                dst.put_u8(MSG_TILE_CHANGED);
                dst.put_u32_le(position.x);
                dst.put_u32_le(position.y);
                dst.put_u32_le(position.z);
                encode_tile(items.as_ref(), dst);
            }
            ServerMessage::PlayerWalkAck { position, tiles } => {
                dst.put_u8(MSG_PLAYER_WALK_ACK);
                encode_position(position, dst);
                for t in tiles.iter() {
                    encode_tile(t, dst);
                }
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
                title,
                items,
            } => {
                dst.put_u8(MSG_OPEN_CONTAINER);
                dst.put_u16_le(container_id);
                dst.put_u8(capacity);
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
        }

        let payload_len = (dst.len() - len_offset - 2) as u16;
        dst[len_offset..len_offset + 2].copy_from_slice(&payload_len.to_le_bytes());

        Ok(())
    }
}

fn encode_position(pos: Position, dst: &mut BytesMut) {
    dst.put_u32_le(pos.x);
    dst.put_u32_le(pos.y);
    dst.put_u32_le(pos.z);
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
    }
}
