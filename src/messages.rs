use thiserror::Error;
use tokio_util::{
    bytes::{Buf, BufMut, BytesMut},
    codec::{Decoder, Encoder},
};

use crate::{
    constants::{MAX_VISIBLE_ITEMS, PLAYER_VIEWPORT_HEIGHT, PLAYER_VIEWPORT_WIDTH},
    entities::items::ItemId,
    entities::map::Position,
};

pub type ItemStack = [Option<(ItemId, u8)>; MAX_VISIBLE_ITEMS];

#[derive(Clone, Debug)]
pub enum Direction {
    North,
    East,
    West,
    South,
    NorthEast,
    SouthEast,
    NorthWest,
    SouthWest,
}

// client
const MSG_LOGIN: u8 = 0x01;
const MSG_MOVE_PLAYER: u8 = 0x02;
const MSG_MOVE_ITEM: u8 = 0x03;

#[derive(Clone, Debug)]
pub enum ClientMessage {
    Login {
        character_id: u32,
        auth_token: String,
    },
    MovePlayer {
        direction: Direction,
    },
    MoveItem {
        from: Position,
        item_id: ItemId,
        amount: u8,
        stack_index: u16,
        to: Position,
    },
}

// server
const MSG_DESCRIBE_MAP: u8 = 0x01;
const MSG_TILE_CHANGED: u8 = 0x02;
const MSG_PLAYER_WALK: u8 = 0x03;
const MSG_PLAYER_POS: u8 = 0x04;

#[derive(Clone, Debug)]
pub enum ServerMessage {
    DescribeMap {
        tiles: Box<[ItemStack; PLAYER_VIEWPORT_HEIGHT * PLAYER_VIEWPORT_WIDTH]>,
    },
    TileChanged {
        position: Position,
        items: Box<ItemStack>,
    },
    PlayerWalk {
        direction: Direction,
        tiles: Box<[ItemStack]>,
    },
    PlayerPosition {
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
            ServerMessage::PlayerWalk { direction, tiles } => {
                dst.put_u8(MSG_PLAYER_WALK);
                encode_direction(&direction, dst);
                for t in tiles.iter() {
                    encode_tile(t, dst);
                }
            }
            ServerMessage::PlayerPosition { position } => {
                dst.put_u8(MSG_PLAYER_POS);
                encode_position(position, dst);
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
