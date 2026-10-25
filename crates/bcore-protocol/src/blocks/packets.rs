//! Native 26.2 block-action and unpatched creative-hotbar wire formats.
use std::io::{Cursor, Read};

use bcore_core::{varint::encode_varint, BlockPos, ChunkPos};

use crate::packet::{read_varint, write_packet, PacketError};

pub const SB_PLAYER_ACTION: i32 = 41;
pub const SB_SET_CARRIED_ITEM: i32 = 53;
pub const SB_SET_CREATIVE_SLOT: i32 = 56;
pub const SB_USE_ITEM_ON: i32 = 66;
pub const CB_BLOCK_ACK: i32 = 4;
pub const CB_BLOCK_UPDATE: i32 = 8;
pub const CB_LIGHT_UPDATE: i32 = 48;
pub const CB_PLAYER_INVENTORY: i32 = 108;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimpleStack {
    pub item: u32,
    pub count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockHit {
    pub pos: BlockPos,
    pub face: u8,
    pub offsets: [f32; 3],
    pub inside: bool,
    pub world_border: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockPacket {
    Action {
        action: u8,
        pos: BlockPos,
        face: u8,
        sequence: i32,
    },
    Place {
        hand: u8,
        hit: BlockHit,
        sequence: i32,
    },
    Carried {
        slot: u8,
    },
    Creative {
        slot: u8,
        stack: Option<SimpleStack>,
    },
}

fn bytes<const N: usize>(input: &mut Cursor<&[u8]>) -> Result<[u8; N], PacketError> {
    let mut bytes = [0; N];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn bounded(
    input: &mut Cursor<&[u8]>,
    maximum: i32,
    error: &'static str,
) -> Result<i32, PacketError> {
    let value = read_varint(input)?;
    if !(0..=maximum).contains(&value) {
        return Err(PacketError::Malformed(error));
    }
    Ok(value)
}

fn position(input: &mut Cursor<&[u8]>) -> Result<BlockPos, PacketError> {
    let packed = i64::from_be_bytes(bytes(input)?);
    Ok(BlockPos::new(
        (packed >> 38) as i32,
        ((packed << 52) >> 52) as i32,
        ((packed << 26) >> 38) as i32,
    ))
}

fn boolean(input: &mut Cursor<&[u8]>) -> Result<bool, PacketError> {
    match bytes::<1>(input)?[0] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(PacketError::Malformed("invalid block-hit boolean")),
    }
}

fn creative(input: &mut Cursor<&[u8]>) -> Result<BlockPacket, PacketError> {
    let slot = i16::from_be_bytes(bytes(input)?);
    if !(36..=44).contains(&slot) {
        return Err(PacketError::Malformed(
            "unsupported creative inventory slot",
        ));
    }
    let count = bounded(input, 64, "invalid creative stack count")? as u8;
    let stack = if count == 0 {
        None
    } else {
        let item = bounded(
            input,
            bcore_registry::vanilla().registries["ITEM"].len() as i32 - 1,
            "invalid creative item ID",
        )? as u32;
        let added = read_varint(input)?;
        let removed = read_varint(input)?;
        if added != 0 || removed != 0 {
            return Err(PacketError::Malformed(
                "patched creative stacks are not supported",
            ));
        }
        Some(SimpleStack { item, count })
    };
    Ok(BlockPacket::Creative {
        slot: (slot - 36) as u8,
        stack,
    })
}

fn place(input: &mut Cursor<&[u8]>) -> Result<BlockPacket, PacketError> {
    let hand = bounded(input, 1, "invalid interaction hand")? as u8;
    let pos = position(input)?;
    let face = bounded(input, 5, "invalid block face")? as u8;
    let offsets = [
        f32::from_be_bytes(bytes(input)?),
        f32::from_be_bytes(bytes(input)?),
        f32::from_be_bytes(bytes(input)?),
    ];
    let inside = boolean(input)?;
    let world_border = boolean(input)?;
    let sequence = bounded(input, i32::MAX, "negative block sequence")?;
    Ok(BlockPacket::Place {
        hand,
        hit: BlockHit {
            pos,
            face,
            offsets,
            inside,
            world_border,
        },
        sequence,
    })
}

pub fn parse(id: i32, data: &[u8]) -> Result<Option<BlockPacket>, PacketError> {
    let mut input = Cursor::new(data);
    let packet = match id {
        SB_PLAYER_ACTION => {
            let action = bounded(&mut input, 7, "invalid player action")? as u8;
            let pos = position(&mut input)?;
            let face = bytes::<1>(&mut input)?[0];
            if face > 5 {
                return Err(PacketError::Malformed("invalid block face"));
            }
            let sequence = bounded(&mut input, i32::MAX, "negative block sequence")?;
            BlockPacket::Action {
                action,
                pos,
                face,
                sequence,
            }
        }
        SB_USE_ITEM_ON => place(&mut input)?,
        SB_SET_CARRIED_ITEM => {
            let slot = i16::from_be_bytes(bytes(&mut input)?);
            if !(0..=8).contains(&slot) {
                return Err(PacketError::Malformed("invalid carried slot"));
            }
            BlockPacket::Carried { slot: slot as u8 }
        }
        SB_SET_CREATIVE_SLOT => creative(&mut input)?,
        _ => return Ok(None),
    };
    if input.position() != data.len() as u64 {
        return Err(PacketError::Malformed("trailing block-action payload"));
    }
    Ok(Some(packet))
}

fn frame(id: i32, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    write_packet(&mut frame, id, payload);
    frame
}

pub fn encode_ack(sequence: i32) -> Vec<u8> {
    let mut payload = Vec::new();
    encode_varint(sequence, &mut payload);
    frame(CB_BLOCK_ACK, &payload)
}

pub fn encode_block_update(pos: BlockPos, state: u32) -> Vec<u8> {
    let packed = ((pos.x as u64 & 0x3ff_ffff) << 38)
        | ((pos.z as u64 & 0x3ff_ffff) << 12)
        | (pos.y as u64 & 0xfff);
    let mut payload = packed.to_be_bytes().to_vec();
    encode_varint(state as i32, &mut payload);
    frame(CB_BLOCK_UPDATE, &payload)
}

pub fn encode_inventory(slot: u8, stack: Option<SimpleStack>) -> Vec<u8> {
    let mut payload = Vec::new();
    encode_varint(i32::from(slot), &mut payload);
    if let Some(stack) = stack {
        encode_varint(i32::from(stack.count), &mut payload);
        encode_varint(stack.item as i32, &mut payload);
        payload.extend_from_slice(&[0, 0]);
    } else {
        encode_varint(0, &mut payload);
    }
    frame(CB_PLAYER_INVENTORY, &payload)
}

pub(super) fn encode_light(pos: ChunkPos, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    encode_varint(pos.x, &mut payload);
    encode_varint(pos.z, &mut payload);
    payload.extend_from_slice(data);
    frame(CB_LIGHT_UPDATE, &payload)
}

pub fn adjacent(pos: BlockPos, face: u8) -> BlockPos {
    let [x, y, z] = match face {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        5 => [1, 0, 0],
        _ => unreachable!("validated block face"),
    };
    BlockPos::new(pos.x + x, pos.y + y, pos.z + z)
}

pub fn hit_is_valid(hit: BlockHit) -> bool {
    !hit.world_border
        && hit
            .offsets
            .into_iter()
            .all(|value| value.is_finite() && (f64::from(value) - 0.5).abs() < 1.0000001)
}

pub fn within_reach(eyes: [f64; 3], pos: BlockPos, range: f64) -> bool {
    if !eyes.into_iter().all(f64::is_finite) || !range.is_finite() || range <= 0.0 {
        return false;
    }
    let mut distance = 0.0;
    for (point, lower) in eyes.into_iter().zip([pos.x, pos.y, pos.z]) {
        let lower = f64::from(lower);
        let difference = (lower - point).max(0.0).max(point - (lower + 1.0));
        distance += difference * difference;
    }
    distance < range * range
}
