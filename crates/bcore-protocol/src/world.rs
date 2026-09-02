//! Player position tracking and view-distance chunk streaming.
//!
//! The play loop reads the serverbound movement packets, keeps the player's
//! position up to date and, whenever the player crosses into a new chunk,
//! streams the chunks that entered the view distance (and unloads the ones that
//! left it). Chunk columns are generated natively by [`crate::chunk`].

use std::collections::BTreeSet;
use std::io::Write;

use bcore_core::varint::encode_varint;

use crate::chunk::flat_chunk_payload;
use crate::packet::{write_packet, PacketError};

/// Serverbound: `position` — x/y/z + movement flags.
pub const SB_POSITION: i32 = 0x1e;
/// Serverbound: `position_look` — x/y/z + yaw/pitch + movement flags.
pub const SB_POSITION_LOOK: i32 = 0x1f;
/// Serverbound: `look` — yaw/pitch + movement flags.
pub const SB_LOOK: i32 = 0x20;
/// Serverbound: `teleport_confirm`.
pub const SB_TELEPORT_CONFIRM: i32 = 0x00;
/// Serverbound: `chunk_batch_received` — the client's requested chunks/tick.
pub const SB_CHUNK_BATCH_RECEIVED: i32 = 0x0b;
/// Serverbound: `keep_alive` response.
pub const SB_KEEP_ALIVE: i32 = 0x1c;
/// Serverbound: `ping_request`.
pub const SB_PING_REQUEST: i32 = 0x26;
/// Serverbound: `player_loaded`.
pub const SB_PLAYER_LOADED: i32 = 0x2c;

/// Clientbound: `chunk_batch_finished` — carries the batch size.
pub const CB_CHUNK_BATCH_FINISHED: i32 = 0x0b;
/// Clientbound: `chunk_batch_start` — empty payload.
pub const CB_CHUNK_BATCH_START: i32 = 0x0c;
/// Clientbound: `unload_chunk` — note the z-before-x field order.
pub const CB_UNLOAD_CHUNK: i32 = 0x25;
/// Clientbound: `map_chunk`.
pub const CB_MAP_CHUNK: i32 = 0x2d;
/// Clientbound: `ping_response`.
pub const CB_PING_RESPONSE: i32 = 0x3e;
/// Clientbound: `update_view_position` — the chunk the client should centre on.
pub const CB_UPDATE_VIEW_POSITION: i32 = 0x5e;

/// View distance in chunks, matching the `viewDistance` announced at join.
pub const VIEW_DISTANCE: i32 = 2;

/// The chunk containing a world coordinate (floor division by 16).
pub fn chunk_coord(world: f64) -> i32 {
    (world.floor() as i64 >> 4) as i32
}

/// Chunk positions within `radius` of `(cx, cz)`, ordered by distance from the
/// centre then by coordinate. Deterministic: no hash iteration.
pub fn chunks_in_view(cx: i32, cz: i32, radius: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::with_capacity(((2 * radius + 1) * (2 * radius + 1)) as usize);
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            out.push((cx + dx, cz + dz));
        }
    }
    // Closest-first (vanilla streams outwards); ties broken by coordinate so the
    // ordering is fully determined.
    out.sort_by_key(|&(x, z)| {
        let (dx, dz) = ((x - cx).abs(), (z - cz).abs());
        (dx.max(dz), dx + dz, x, z)
    });
    out
}

/// Tracks a player's position and which chunks the client currently holds.
#[derive(Debug, Clone)]
pub struct PlayerView {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    /// Chunks the client has been sent and has not been told to unload.
    loaded: BTreeSet<(i32, i32)>,
    view_distance: i32,
}

impl PlayerView {
    /// A view centred on the given spawn position, with no chunks loaded yet.
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self {
            x,
            y,
            z,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            loaded: BTreeSet::new(),
            view_distance: VIEW_DISTANCE,
        }
    }

    /// Override the streaming radius (chunks).
    pub fn with_view_distance(mut self, view_distance: i32) -> Self {
        self.view_distance = view_distance;
        self
    }

    /// The chunk the player currently occupies.
    pub fn chunk(&self) -> (i32, i32) {
        (chunk_coord(self.x), chunk_coord(self.z))
    }

    /// Chunks the client holds, in deterministic order.
    pub fn loaded_chunks(&self) -> impl Iterator<Item = &(i32, i32)> {
        self.loaded.iter()
    }

    /// Mark a chunk as already delivered (used to adopt the join-time batch).
    pub fn mark_loaded(&mut self, x: i32, z: i32) {
        self.loaded.insert((x, z));
    }

    /// Apply a serverbound movement packet, returning `true` if it was one.
    ///
    /// Malformed/short payloads are ignored rather than dropping the connection:
    /// a bad movement packet should not disconnect a player.
    pub fn apply_movement(&mut self, packet_id: i32, data: &[u8]) -> bool {
        match packet_id {
            SB_POSITION if data.len() >= 24 => {
                self.read_xyz(data);
                self.on_ground = data.get(24).is_some_and(|f| f & 0x01 != 0);
                true
            }
            SB_POSITION_LOOK if data.len() >= 32 => {
                self.read_xyz(data);
                self.yaw = read_f32(data, 24);
                self.pitch = read_f32(data, 28);
                self.on_ground = data.get(32).is_some_and(|f| f & 0x01 != 0);
                true
            }
            SB_LOOK if data.len() >= 8 => {
                self.yaw = read_f32(data, 0);
                self.pitch = read_f32(data, 4);
                self.on_ground = data.get(8).is_some_and(|f| f & 0x01 != 0);
                true
            }
            SB_POSITION | SB_POSITION_LOOK | SB_LOOK => true, // short: ignore payload
            _ => false,
        }
    }

    fn read_xyz(&mut self, data: &[u8]) {
        self.x = read_f64(data, 0);
        self.y = read_f64(data, 8);
        self.z = read_f64(data, 16);
    }

    /// Chunks inside the view distance that the client does not have yet.
    pub fn missing_chunks(&self) -> Vec<(i32, i32)> {
        let (cx, cz) = self.chunk();
        chunks_in_view(cx, cz, self.view_distance)
            .into_iter()
            .filter(|pos| !self.loaded.contains(pos))
            .collect()
    }

    /// Chunks the client holds that have left the view distance.
    pub fn stale_chunks(&self) -> Vec<(i32, i32)> {
        let (cx, cz) = self.chunk();
        self.loaded
            .iter()
            .copied()
            .filter(|&(x, z)| {
                (x - cx).abs() > self.view_distance || (z - cz).abs() > self.view_distance
            })
            .collect()
    }

    /// Stream the chunks that entered the view distance and unload the rest.
    ///
    /// Sends `update_view_position`, then a `chunk_batch_start` /
    /// `map_chunk`* / `chunk_batch_finished` batch, then `unload_chunk` for
    /// everything that fell out of range. Returns the number of chunks sent.
    pub fn stream_chunks<W: Write>(&mut self, out: &mut W) -> Result<usize, PacketError> {
        let missing = self.missing_chunks();
        let stale = self.stale_chunks();
        if missing.is_empty() && stale.is_empty() {
            return Ok(0);
        }

        let (cx, cz) = self.chunk();
        let mut buf = Vec::new();

        let mut center = Vec::new();
        encode_varint(cx, &mut center);
        encode_varint(cz, &mut center);
        write_packet(&mut buf, CB_UPDATE_VIEW_POSITION, &center);

        if !missing.is_empty() {
            write_packet(&mut buf, CB_CHUNK_BATCH_START, &[]);
            for &(x, z) in &missing {
                write_packet(&mut buf, CB_MAP_CHUNK, &flat_chunk_payload(x, z));
                self.loaded.insert((x, z));
            }
            let mut size = Vec::new();
            encode_varint(missing.len() as i32, &mut size);
            write_packet(&mut buf, CB_CHUNK_BATCH_FINISHED, &size);
        }

        for (x, z) in stale {
            // unload_chunk is (chunkZ, chunkX) on the wire.
            let mut payload = Vec::with_capacity(8);
            payload.extend_from_slice(&z.to_be_bytes());
            payload.extend_from_slice(&x.to_be_bytes());
            write_packet(&mut buf, CB_UNLOAD_CHUNK, &payload);
            self.loaded.remove(&(x, z));
        }

        out.write_all(&buf)?;
        Ok(missing.len())
    }
}

fn read_f64(data: &[u8], at: usize) -> f64 {
    f64::from_be_bytes(data[at..at + 8].try_into().expect("checked length"))
}

fn read_f32(data: &[u8], at: usize) -> f32 {
    f32::from_be_bytes(data[at..at + 4].try_into().expect("checked length"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_coords_floor_towards_negative_infinity() {
        assert_eq!(chunk_coord(0.0), 0);
        assert_eq!(chunk_coord(15.9), 0);
        assert_eq!(chunk_coord(16.0), 1);
        assert_eq!(chunk_coord(-0.5), -1);
        assert_eq!(chunk_coord(-16.0), -1);
        assert_eq!(chunk_coord(-16.1), -2);
        assert_eq!(chunk_coord(-3.5), -1);
    }

    #[test]
    fn view_is_a_square_ordered_closest_first_and_deterministic() {
        let view = chunks_in_view(0, -1, 2);
        assert_eq!(view.len(), 25);
        assert_eq!(view[0], (0, -1), "centre first");
        assert_eq!(chunks_in_view(0, -1, 2), view, "deterministic ordering");
        // Ring 1 (8 chunks) precedes ring 2 (16 chunks).
        for pos in &view[1..9] {
            assert_eq!((pos.0 - 0).abs().max((pos.1 + 1).abs()), 1);
        }
        for pos in &view[9..] {
            assert_eq!((pos.0 - 0).abs().max((pos.1 + 1).abs()), 2);
        }
    }

    #[test]
    fn position_packet_updates_the_tracked_chunk() {
        let mut view = PlayerView::new(10.5, -60.0, -3.5);
        assert_eq!(view.chunk(), (0, -1));
        let mut data = Vec::new();
        data.extend_from_slice(&40.0f64.to_be_bytes());
        data.extend_from_slice(&(-60.0f64).to_be_bytes());
        data.extend_from_slice(&(-3.5f64).to_be_bytes());
        data.push(0x01); // onGround
        assert!(view.apply_movement(SB_POSITION, &data));
        assert_eq!(view.x, 40.0);
        assert!(view.on_ground);
        assert_eq!(view.chunk(), (2, -1));
    }

    #[test]
    fn position_look_updates_rotation_too() {
        let mut view = PlayerView::new(0.0, 0.0, 0.0);
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f64.to_be_bytes());
        data.extend_from_slice(&2.0f64.to_be_bytes());
        data.extend_from_slice(&3.0f64.to_be_bytes());
        data.extend_from_slice(&90.0f32.to_be_bytes());
        data.extend_from_slice(&(-12.5f32).to_be_bytes());
        data.push(0x00);
        assert!(view.apply_movement(SB_POSITION_LOOK, &data));
        assert_eq!((view.x, view.y, view.z), (1.0, 2.0, 3.0));
        assert_eq!(view.yaw, 90.0);
        assert_eq!(view.pitch, -12.5);
        assert!(!view.on_ground);
    }

    #[test]
    fn look_only_changes_rotation() {
        let mut view = PlayerView::new(5.0, 6.0, 7.0);
        let mut data = Vec::new();
        data.extend_from_slice(&45.0f32.to_be_bytes());
        data.extend_from_slice(&10.0f32.to_be_bytes());
        data.push(0x01);
        assert!(view.apply_movement(SB_LOOK, &data));
        assert_eq!((view.x, view.y, view.z), (5.0, 6.0, 7.0));
        assert_eq!(view.yaw, 45.0);
    }

    #[test]
    fn non_movement_packets_and_short_payloads_are_safe() {
        let mut view = PlayerView::new(1.0, 2.0, 3.0);
        assert!(!view.apply_movement(SB_KEEP_ALIVE, &[0; 8]));
        // Truncated position: accepted as a movement packet but ignored.
        assert!(view.apply_movement(SB_POSITION, &[0, 1, 2]));
        assert_eq!((view.x, view.y, view.z), (1.0, 2.0, 3.0));
    }

    #[test]
    fn streaming_sends_the_full_view_then_nothing_until_the_player_moves() {
        let mut view = PlayerView::new(10.5, -60.0, -3.5);
        let mut out = Vec::new();
        assert_eq!(view.stream_chunks(&mut out).expect("stream"), 25);
        assert!(!out.is_empty());
        // Idempotent while the player stays put.
        let mut again = Vec::new();
        assert_eq!(view.stream_chunks(&mut again).expect("stream"), 0);
        assert!(again.is_empty());
    }

    #[test]
    fn crossing_a_chunk_border_streams_only_the_new_column() {
        let mut view = PlayerView::new(10.5, -60.0, -3.5);
        view.stream_chunks(&mut Vec::new()).expect("initial");
        assert_eq!(view.chunk(), (0, -1));

        // Step east into chunk (1, -1): one new 5-chunk column, one stale column.
        let mut data = Vec::new();
        data.extend_from_slice(&26.5f64.to_be_bytes());
        data.extend_from_slice(&(-60.0f64).to_be_bytes());
        data.extend_from_slice(&(-3.5f64).to_be_bytes());
        data.push(0x01);
        view.apply_movement(SB_POSITION, &data);
        assert_eq!(view.chunk(), (1, -1));
        assert_eq!(view.missing_chunks().len(), 5);
        assert_eq!(view.stale_chunks().len(), 5);

        let mut out = Vec::new();
        assert_eq!(view.stream_chunks(&mut out).expect("stream"), 5);
        // The view is still exactly 25 chunks after the shift.
        assert_eq!(view.loaded_chunks().count(), 25);
        assert!(view.missing_chunks().is_empty());
        assert!(view.stale_chunks().is_empty());
    }

    #[test]
    fn adopting_the_join_batch_avoids_resending_it() {
        let mut view = PlayerView::new(10.5, -60.0, -3.5);
        for &(x, z) in &chunks_in_view(0, -1, VIEW_DISTANCE) {
            view.mark_loaded(x, z);
        }
        let mut out = Vec::new();
        assert_eq!(view.stream_chunks(&mut out).expect("stream"), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn a_long_jump_replaces_the_whole_view() {
        let mut view = PlayerView::new(10.5, -60.0, -3.5);
        view.stream_chunks(&mut Vec::new()).expect("initial");
        let mut data = Vec::new();
        data.extend_from_slice(&1000.0f64.to_be_bytes());
        data.extend_from_slice(&(-60.0f64).to_be_bytes());
        data.extend_from_slice(&1000.0f64.to_be_bytes());
        data.push(0x01);
        view.apply_movement(SB_POSITION, &data);
        assert_eq!(view.missing_chunks().len(), 25);
        assert_eq!(view.stale_chunks().len(), 25);
        let mut out = Vec::new();
        assert_eq!(view.stream_chunks(&mut out).expect("stream"), 25);
        assert_eq!(view.loaded_chunks().count(), 25);
    }
}
