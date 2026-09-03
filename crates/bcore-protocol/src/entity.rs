//! Basic entity state and clientbound entity packet encoders for protocol 776.

use bcore_core::varint::encode_varint;

use crate::packet::write_packet;

pub const CB_SPAWN_ENTITY: i32 = 0x01;
pub const CB_ENTITY_METADATA: i32 = 0x63;
pub const CB_REMOVE_ENTITIES: i32 = 0x4d; // entity_destroy in protocol_776
pub const CB_ENTITY_TELEPORT: i32 = 0x7d;

/// Monotonically allocates positive protocol entity ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityIdAllocator {
    next: i32,
}

impl EntityIdAllocator {
    pub fn new(first: i32) -> Self {
        assert!(first >= 0, "entity ids must be non-negative");
        Self { next: first }
    }

    pub fn allocate(&mut self) -> i32 {
        let id = self.next;
        self.next = self.next.checked_add(1).expect("entity id exhausted");
        id
    }
}

impl Default for EntityIdAllocator {
    fn default() -> Self {
        Self::new(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemEntity {
    pub id: i32,
    pub position: Position,
    pub owner: Option<[u8; 16]>,
    pub age: i16,
    /// Minecraft item entity type id (item = 2 in the entity registry).
    pub entity_type: i32,
}

impl ItemEntity {
    pub fn new(id: i32, position: Position) -> Self {
        Self {
            id,
            position,
            owner: None,
            age: 0,
            entity_type: 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobKind {
    Zombie,
    Cow,
}

impl MobKind {
    pub fn entity_type(self) -> i32 {
        match self {
            Self::Cow => 30,
            Self::Zombie => 151,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MobEntity {
    pub id: i32,
    pub kind: MobKind,
    pub position: Position,
    pub health: f32,
}

impl MobEntity {
    pub fn new(id: i32, kind: MobKind, position: Position) -> Self {
        Self {
            id,
            kind,
            position,
            health: 20.0,
        }
    }
}

fn angle(value: f32) -> i8 {
    (value * 256.0 / 360.0) as i8
}

/// Encode clientbound `spawn_entity`: id, UUID, type, position, velocity,
/// three angles and object data. UUID is supplied to keep output deterministic.
pub fn encode_spawn_entity(
    id: i32,
    uuid: [u8; 16],
    entity_type: i32,
    position: Position,
) -> Vec<u8> {
    let mut data = Vec::new();
    encode_varint(id, &mut data);
    data.extend_from_slice(&uuid);
    encode_varint(entity_type, &mut data);
    data.extend_from_slice(&position.x.to_be_bytes());
    data.extend_from_slice(&position.y.to_be_bytes());
    data.extend_from_slice(&position.z.to_be_bytes());
    data.extend_from_slice(&[0; 6]); // lpVec3 velocity: three i16 values
    data.extend_from_slice(&[0, 0, 0]); // pitch, yaw, headPitch
    encode_varint(0, &mut data); // objectData
    let mut packet = Vec::new();
    write_packet(&mut packet, CB_SPAWN_ENTITY, &data);
    packet
}

/// A raw metadata entry. `value` must already use the selected protocol type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEntry {
    pub index: u8,
    pub type_id: i32,
    pub value: Vec<u8>,
}

/// Encode `entity_metadata`; entries are terminated by 0xff.
pub fn encode_entity_metadata(id: i32, entries: &[MetadataEntry]) -> Vec<u8> {
    let mut data = Vec::new();
    encode_varint(id, &mut data);
    for entry in entries {
        data.push(entry.index);
        encode_varint(entry.type_id, &mut data);
        data.extend_from_slice(&entry.value);
    }
    data.push(0xff);
    let mut packet = Vec::new();
    write_packet(&mut packet, CB_ENTITY_METADATA, &data);
    packet
}

pub fn encode_entity_teleport(
    id: i32,
    position: Position,
    yaw: f32,
    pitch: f32,
    on_ground: bool,
) -> Vec<u8> {
    let mut data = Vec::new();
    encode_varint(id, &mut data);
    data.extend_from_slice(&position.x.to_be_bytes());
    data.extend_from_slice(&position.y.to_be_bytes());
    data.extend_from_slice(&position.z.to_be_bytes());
    data.extend_from_slice(&[angle(yaw) as u8, angle(pitch) as u8]);
    data.push(u8::from(on_ground));
    let mut packet = Vec::new();
    write_packet(&mut packet, CB_ENTITY_TELEPORT, &data);
    packet
}

pub fn encode_remove_entities(ids: &[i32]) -> Vec<u8> {
    let mut data = Vec::new();
    encode_varint(ids.len() as i32, &mut data);
    for &id in ids {
        encode_varint(id, &mut data);
    }
    let mut packet = Vec::new();
    write_packet(&mut packet, CB_REMOVE_ENTITIES, &data);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::read_frame;
    use std::io::Cursor;

    fn body(packet: Vec<u8>, id: i32) -> Vec<u8> {
        let (actual, payload) = read_frame(&mut Cursor::new(packet)).unwrap();
        assert_eq!(actual, id);
        payload
    }

    #[test]
    fn allocator_is_monotonic() {
        let mut a = EntityIdAllocator::new(7);
        assert_eq!([a.allocate(), a.allocate(), a.allocate()], [7, 8, 9]);
    }

    #[test]
    fn spawn_has_protocol_fields() {
        let p = Position {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        let b = body(
            encode_spawn_entity(4, [0xabu8; 16], 151, p),
            CB_SPAWN_ENTITY,
        );
        assert_eq!(&b[..1], &[4]);
        assert_eq!(&b[1..17], &[0xab; 16]);
        assert_eq!(b.len(), 1 + 16 + 2 + 24 + 6 + 3 + 1);
    }

    #[test]
    fn metadata_is_terminated() {
        let b = body(
            encode_entity_metadata(
                3,
                &[MetadataEntry {
                    index: 0,
                    type_id: 0,
                    value: vec![1],
                }],
            ),
            CB_ENTITY_METADATA,
        );
        assert_eq!(&b[b.len() - 2..], &[1, 0xff]);
    }

    #[test]
    fn teleport_and_remove_encode_ids() {
        let p = Position {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        assert_eq!(
            body(
                encode_entity_teleport(5, p, 0.0, 0.0, true),
                CB_ENTITY_TELEPORT
            )[0],
            5
        );
        let b = body(encode_remove_entities(&[5, 130]), CB_REMOVE_ENTITIES);
        assert_eq!(b, vec![2, 5, 130, 1]);
    }
}
