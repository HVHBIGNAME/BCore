//! Entity state and native-checked clientbound entity packets for protocol 775.

use bcore_core::varint::encode_varint;
use bcore_worldgen::generated_entity::GeneratedEntity;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::packet::write_packet;

pub const CB_SPAWN_ENTITY: i32 = 0x01;
pub const CB_ENTITY_METADATA: i32 = 0x63;
pub const CB_REMOVE_ENTITIES: i32 = 0x4d;
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
    /// Minecraft item entity type id (71 in the 26.1 registry).
    pub entity_type: i32,
}

impl ItemEntity {
    pub fn new(id: i32, position: Position) -> Self {
        Self {
            id,
            position,
            owner: None,
            age: 0,
            entity_type: 71,
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
            Self::Zombie => 150,
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
    data.push(0); // Native LpVec3 encodes the zero vector with one byte.
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
    data.extend_from_slice(&[0; 24]); // delta movement: three f64 values
    data.extend_from_slice(&yaw.to_be_bytes());
    data.extend_from_slice(&pitch.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes()); // no relative fields
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

/// Runtime identity is shared by viewers, independently of feature-placement RNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedEntity {
    pub id: i32,
    pub uuid: [u8; 16],
    pub generated: GeneratedEntity,
}

static NEXT_GENERATED_ID: AtomicI32 = AtomicI32::new(crate::join::PLAY_LOGIN_ENTITY_ID + 1);

impl TrackedEntity {
    pub fn new(seed: i64, chunk: (i32, i32), ordinal: usize, generated: GeneratedEntity) -> Self {
        let id = NEXT_GENERATED_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("entity id exhausted");
        let mut digest = Sha256::new();
        digest.update(b"BCore generated entity identity v1\0");
        digest.update(seed.to_le_bytes());
        digest.update(chunk.0.to_le_bytes());
        digest.update(chunk.1.to_le_bytes());
        digest.update((ordinal as u64).to_le_bytes());
        digest.update(generated.type_id().to_le_bytes());
        for coordinate in generated.block_pos() {
            digest.update(coordinate.to_le_bytes());
        }
        match &generated {
            GeneratedEntity::ChestMinecart { loot_seed, .. } => {
                digest.update(loot_seed.to_le_bytes())
            }
        }
        let mut uuid: [u8; 16] = digest.finalize()[..16].try_into().unwrap();
        uuid[6] = (uuid[6] & 0x0f) | 0x80; // UUID v8: application-defined SHA-256 identity.
        uuid[8] = (uuid[8] & 0x3f) | 0x80;
        Self {
            id,
            uuid,
            generated,
        }
    }

    pub fn spawn_packet(&self) -> Vec<u8> {
        let [x, y, z] = self.generated.position();
        encode_spawn_entity(
            self.id,
            self.uuid,
            self.generated.type_id() as i32,
            Position { x, y, z },
        )
    }
}

pub(crate) type TrackedEntities = Arc<[TrackedEntity]>;

/// Payloads and active views own the entities; this index holds only weak refs.
#[derive(Debug, Default)]
pub(crate) struct EntityTracker {
    chunks: Mutex<HashMap<(i32, i32), Weak<[TrackedEntity]>>>,
}

impl EntityTracker {
    pub fn for_chunk(
        &self,
        seed: i64,
        chunk: (i32, i32),
        data: &[GeneratedEntity],
    ) -> TrackedEntities {
        if data.is_empty() {
            return Arc::from([]);
        }
        let mut chunks = self.chunks.lock().expect("entity tracker lock");
        if let Some(existing) = chunks.get(&chunk).and_then(Weak::upgrade) {
            if existing.len() == data.len()
                && existing
                    .iter()
                    .zip(data)
                    .all(|(old, new)| old.generated == *new)
            {
                return existing;
            }
        }
        let tracked: TrackedEntities = data
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, data)| TrackedEntity::new(seed, chunk, index, data))
            .collect::<Vec<_>>()
            .into();
        if chunks.len() >= 8192 {
            chunks.retain(|_, entities| entities.strong_count() != 0);
        }
        chunks.insert(chunk, Arc::downgrade(&tracked));
        tracked
    }

    pub fn prune(&self) {
        self.chunks
            .lock()
            .expect("entity tracker lock")
            .retain(|_, entities| entities.strong_count() != 0);
    }
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
        assert_eq!(b.len(), 1 + 16 + 2 + 24 + 1 + 3 + 1);
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
