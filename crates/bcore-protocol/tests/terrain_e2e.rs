//! End-to-end: a real server hands a real client realistic terrain over TCP.
//!
//! This is the test that proves the whole chain works together — generator →
//! `ChunkColumn` → paletted containers → `map_chunk` → socket — rather than each
//! piece in isolation. It starts `server::run` on an ephemeral port (never
//! `bcore.exe`, which may be serving live players), joins through the full
//! login → configuration → play handshake, and then decodes the chunks the server
//! actually sent to assert:
//!
//!   * the world is **not** superflat: surface height varies between chunks,
//!   * the layer stack is real terrain (grass/sand over soil over stone),
//!   * bedrock floors every column and nothing is below it,
//!   * water appears at or below sea level and never above it,
//!   * the player is spawned **on** the terrain, not buried inside it,
//!   * the same chunk is byte-identical when a second player asks for it.

use std::io::{Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use bcore_core::varint::{decode_varint, encode_varint};
use bcore_protocol::chunk::{
    block_state, MAX_INDIRECT_BLOCK_BITS, MIN_Y, SEA_LEVEL, SECTION_BIOMES, SECTION_COUNT,
    SECTION_VOLUME,
};
use bcore_protocol::packet::{read_frame, read_string, write_packet, write_string};
use bcore_protocol::server;
use bcore_protocol::shared::new_shared_server;

const CB_MAP_CHUNK: i32 = 0x2d;
const CB_CHUNK_BATCH_FINISHED: i32 = 0x0b;
const CB_POSITION: i32 = 0x48;
const CB_KEEP_ALIVE: i32 = 0x2c;
const CB_KICK_DISCONNECT: i32 = 0x20;
const SB_TELEPORT_CONFIRM: i32 = 0x00;
const SB_CHUNK_BATCH_RECEIVED: i32 = 0x0b;
const SB_KEEP_ALIVE: i32 = 0x1c;
const SB_PLAYER_LOADED: i32 = 0x2c;

// ------------------------------------------------------------ tiny client ---

fn send(stream: &mut TcpStream, id: i32, data: &[u8]) {
    let mut out = Vec::new();
    write_packet(&mut out, id, data);
    stream.write_all(&out).expect("write packet");
}

/// A joined client plus every chunk payload the server sent it.
struct Joined {
    chunks: Vec<(i32, i32, Vec<u8>)>,
    spawn: (f64, f64, f64),
}

/// Run the full handshake and collect the join-time chunk batch.
fn join(addr: SocketAddr, name: &str, uuid_byte: u8) -> Joined {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");

    // Handshake -> login (protocol 776).
    let mut hs = Vec::new();
    encode_varint(776, &mut hs);
    write_string("127.0.0.1", &mut hs);
    hs.extend_from_slice(&addr.port().to_be_bytes());
    encode_varint(2, &mut hs);
    send(&mut stream, 0x00, &hs);

    let mut ls = Vec::new();
    write_string(name, &mut ls);
    ls.extend_from_slice(&[uuid_byte; 16]);
    send(&mut stream, 0x00, &ls);

    let (pid, data) = read_frame(&mut stream).expect("login success");
    assert_eq!(pid, 0x02, "expected login_success, got {pid:#x}");
    let mut cur = Cursor::new(data);
    let mut uuid = [0u8; 16];
    cur.read_exact(&mut uuid).expect("uuid");
    assert_eq!(read_string(&mut cur, 16).expect("name"), name);

    // login_acknowledged -> configuration.
    send(&mut stream, 0x03, &[]);
    loop {
        let (pid, data) = read_frame(&mut stream).expect("configuration");
        match pid {
            0x0e => send(&mut stream, 0x07, &[0x00]), // known_packs -> empty
            0x00 => {
                // plugin_message (brand): echo it back
                let mut reply = data.clone();
                reply.push(0x00);
                send(&mut stream, 0x01, &reply);
            }
            0x02 => panic!("kicked during configuration"),
            0x03 => break, // finish_configuration
            _ => {}
        }
    }
    send(&mut stream, 0x03, &[]); // acknowledge_finish_configuration

    // Play: collect chunks until the batch is finished.
    let mut chunks = Vec::new();
    let mut spawn = None;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut batches = 0;
    while Instant::now() < deadline {
        let (pid, data) = match read_frame(&mut stream) {
            Ok(frame) => frame,
            Err(_) => break,
        };
        match pid {
            CB_MAP_CHUNK => {
                let x = i32::from_be_bytes(data[0..4].try_into().expect("x"));
                let z = i32::from_be_bytes(data[4..8].try_into().expect("z"));
                chunks.push((x, z, data));
            }
            CB_POSITION => {
                let (tid, n) = decode_varint(&data).expect("teleport id");
                let f = |at: usize| {
                    f64::from_be_bytes(data[n + at..n + at + 8].try_into().expect("f64"))
                };
                spawn = Some((f(0), f(8), f(16)));
                let mut confirm = Vec::new();
                encode_varint(tid, &mut confirm);
                send(&mut stream, SB_TELEPORT_CONFIRM, &confirm);
                send(&mut stream, SB_PLAYER_LOADED, &[]);
            }
            CB_KEEP_ALIVE => send(&mut stream, SB_KEEP_ALIVE, &data),
            CB_CHUNK_BATCH_FINISHED => {
                send(&mut stream, SB_CHUNK_BATCH_RECEIVED, &16.0f32.to_be_bytes());
                batches += 1;
                if !chunks.is_empty() && spawn.is_some() && batches >= 1 {
                    break;
                }
            }
            CB_KICK_DISCONNECT => panic!("server kicked the client during join"),
            _ => {}
        }
    }

    assert!(!chunks.is_empty(), "server sent no chunks");
    Joined {
        chunks,
        spawn: spawn.expect("server never sent a spawn position"),
    }
}

/// Start a BCore server on an ephemeral port; returns its address.
fn start_server() -> SocketAddr {
    bcore_protocol::world::set_view_distance_for_tests(4);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("addr");
    let shared = new_shared_server();
    std::thread::spawn(move || server::run_with_state(listener, shared));
    addr
}

// ---------------------------------------------------------------- decoding ---

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> i32 {
        let (v, n) = decode_varint(&self.data[self.at..]).expect("varint");
        self.at += n;
        v
    }
    fn i16(&mut self) -> i16 {
        let v = i16::from_be_bytes(self.data[self.at..self.at + 2].try_into().expect("i16"));
        self.at += 2;
        v
    }
    fn u8(&mut self) -> u8 {
        let v = self.data[self.at];
        self.at += 1;
        v
    }
    fn longs(&mut self, n: usize) -> Vec<u64> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(u64::from_be_bytes(
                self.data[self.at..self.at + 8].try_into().expect("long"),
            ));
            self.at += 8;
        }
        out
    }
}

fn unpack(bits: u8, longs: &[u64], entries: usize) -> Vec<u32> {
    if bits == 0 {
        return vec![0; entries];
    }
    let per_long = 64 / bits as usize;
    let mask = (1u64 << bits) - 1;
    (0..entries)
        .map(|i| ((longs[i / per_long] >> ((i % per_long) * bits as usize)) & mask) as u32)
        .collect()
}

fn read_container(r: &mut Reader<'_>, entries: usize) -> Vec<u32> {
    let bits = r.u8();
    if bits == 0 {
        return vec![r.varint() as u32; entries];
    }
    let direct = entries == SECTION_VOLUME && bits > MAX_INDIRECT_BLOCK_BITS;
    let mut palette = Vec::new();
    if !direct {
        let len = r.varint();
        for _ in 0..len {
            palette.push(r.varint() as u32);
        }
    }
    let per_long = 64 / bits as usize;
    let longs = r.longs(entries.div_ceil(per_long));
    let raw = unpack(bits, &longs, entries);
    if direct {
        raw
    } else {
        raw.into_iter().map(|i| palette[i as usize]).collect()
    }
}

/// A decoded chunk: all block states in wire order, plus biome ids.
struct Decoded {
    states: Vec<u32>,
    biomes: Vec<u32>,
}

impl Decoded {
    fn get(&self, x: usize, y: i32, z: usize) -> u32 {
        self.states[(y - MIN_Y) as usize * 256 + z * 16 + x]
    }

    /// Topmost non-air Y in a column.
    fn surface(&self, x: usize, z: usize) -> Option<i32> {
        (MIN_Y..MIN_Y + 384)
            .rev()
            .find(|&y| self.get(x, y, z) != block_state::AIR)
    }
}

fn decode(payload: &[u8]) -> Decoded {
    let mut r = Reader {
        data: payload,
        at: 8,
    };
    let count = r.varint();
    for _ in 0..count {
        let _kind = r.varint();
        let n = r.varint() as usize;
        r.longs(n);
    }
    let data_len = r.varint() as usize;
    let end = r.at + data_len;

    let mut states = Vec::with_capacity(SECTION_COUNT * SECTION_VOLUME);
    let mut biomes = Vec::with_capacity(SECTION_COUNT * SECTION_BIOMES);
    while r.at < end {
        let _block_count = r.i16();
        let _fluid_count = r.i16();
        states.extend(read_container(&mut r, SECTION_VOLUME));
        biomes.extend(read_container(&mut r, SECTION_BIOMES));
    }
    assert_eq!(r.at, end, "section data overran");
    Decoded { states, biomes }
}

// ------------------------------------------------------------------- tests ---

#[test]
fn the_server_streams_real_terrain_not_a_flat_world() {
    let addr = start_server();
    let joined = join(addr, "TerrainTester", 0x40);

    assert!(
        joined.chunks.len() >= 9,
        "expected a full view batch, got {} chunks",
        joined.chunks.len()
    );

    // Decode a spread of the received chunks.
    let sample: Vec<(i32, i32, Decoded)> = joined
        .chunks
        .iter()
        .step_by((joined.chunks.len() / 12).max(1))
        .map(|(x, z, payload)| (*x, *z, decode(payload)))
        .collect();
    assert!(sample.len() >= 5, "not enough chunks to judge terrain");

    // 1. Surface height must vary — a flat world would be constant.
    let mut surfaces = Vec::new();
    for (x, z, chunk) in &sample {
        let surface = chunk
            .surface(8, 8)
            .unwrap_or_else(|| panic!("chunk ({x}, {z}) is entirely air"));
        surfaces.push(surface);
    }
    let min = *surfaces.iter().min().expect("surfaces");
    let max = *surfaces.iter().max().expect("surfaces");
    assert!(
        max > min,
        "surface is constant at {min} across {} chunks — still superflat?",
        sample.len()
    );
    // And nowhere near the superflat surface (y = -61).
    assert!(
        min > 0,
        "surface {min} looks like superflat, not generated terrain"
    );

    // 2. Bedrock floors every column of every sampled chunk.
    for (x, z, chunk) in &sample {
        for lz in 0..16 {
            for lx in 0..16 {
                assert_eq!(
                    chunk.get(lx, MIN_Y, lz),
                    block_state::BEDROCK,
                    "chunk ({x}, {z}) column ({lx}, {lz}) has no bedrock floor"
                );
            }
        }
    }

    // 3. The layer stack is real terrain, and water obeys sea level.
    let mut saw_soil_stack = false;
    let mut saw_water = false;
    for (cx, cz, chunk) in &sample {
        for lz in 0..16 {
            for lx in 0..16 {
                let Some(surface) = chunk.surface(lx, lz) else {
                    continue;
                };
                let top = chunk.get(lx, surface, lz);

                if top == block_state::WATER {
                    saw_water = true;
                    // Water never sits above sea level.
                    assert!(
                        surface < SEA_LEVEL,
                        "water at y={surface} is at/above sea level {SEA_LEVEL} \
                         in chunk ({cx}, {cz})"
                    );
                    continue;
                }

                // Dry land: grass/sand/stone/snow on top, never floating soil.
                if top == block_state::GRASS_BLOCK {
                    let under = chunk.get(lx, surface - 1, lz);
                    assert!(
                        under == block_state::DIRT || under == block_state::GRAVEL,
                        "grass at ({lx},{surface},{lz}) of ({cx},{cz}) sits on {under}"
                    );
                    // Deeper down must be stone/ore, not soil.
                    let deep = chunk.get(lx, surface - 10, lz);
                    assert_ne!(deep, block_state::GRASS_BLOCK);
                    assert_ne!(deep, block_state::DIRT, "dirt 10 blocks below the surface");
                    saw_soil_stack = true;
                }
            }
        }
    }
    assert!(
        saw_soil_stack,
        "no grass/dirt/stone stack found in the streamed chunks"
    );
    let _ = saw_water; // water depends on where spawn landed; not required here

    // 4. Biomes are real registry ids.
    for (x, z, chunk) in &sample {
        for &biome in &chunk.biomes {
            assert!(
                biome < 66,
                "chunk ({x}, {z}) used biome id {biome}, outside the 26.2 registry"
            );
        }
    }
}

#[test]
fn the_player_spawns_on_top_of_the_generated_terrain() {
    let addr = start_server();
    let joined = join(addr, "SpawnTester", 0x41);
    let (sx, sy, sz) = joined.spawn;

    // Find the chunk the player spawned in.
    let (cx, cz) = ((sx.floor() as i32) >> 4, (sz.floor() as i32) >> 4);
    let payload = joined
        .chunks
        .iter()
        .find(|(x, z, _)| *x == cx && *z == cz)
        .map(|(_, _, p)| p)
        .unwrap_or_else(|| panic!("server never sent the spawn chunk ({cx}, {cz})"));
    let chunk = decode(payload);

    let lx = (sx.floor() as i32 - (cx << 4)) as usize;
    let lz = (sz.floor() as i32 - (cz << 4)) as usize;
    let surface = chunk
        .surface(lx, lz)
        .expect("the spawn column must have ground");

    // Not buried: the spawn Y is at or above the surface.
    assert!(
        sy >= surface as f64,
        "spawn y={sy} is inside the terrain (surface {surface}) — the player would suffocate"
    );
    // Not absurdly high either.
    assert!(
        sy <= surface as f64 + 3.0,
        "spawn y={sy} is {} blocks above the surface {surface}",
        sy - surface as f64
    );
    // And the block at the player's feet is air or water, not solid rock.
    let at_feet = chunk.get(lx, sy.floor() as i32, lz);
    assert!(
        at_feet == block_state::AIR
            || at_feet == block_state::WATER
            || at_feet == block_state::SHORT_GRASS,
        "the player spawned inside block {at_feet}"
    );
}

#[test]
fn two_players_receive_identical_bytes_for_the_same_chunk() {
    // The world cache must serve the same terrain to everyone, and generation
    // must be deterministic across connections.
    let addr = start_server();
    let first = join(addr, "PlayerOne", 0x42);
    let second = join(addr, "PlayerTwo", 0x43);

    let mut compared = 0;
    for (x, z, payload) in &first.chunks {
        if let Some((_, _, other)) = second.chunks.iter().find(|(ox, oz, _)| ox == x && oz == z) {
            assert_eq!(
                payload, other,
                "chunk ({x}, {z}) differed between two players"
            );
            compared += 1;
        }
    }
    assert!(
        compared >= 9,
        "only {compared} chunks overlapped between the two players"
    );
}

#[test]
fn streamed_chunks_are_all_distinct_terrain() {
    // A flat world sends the same payload (bar coordinates) for every chunk.
    // Real terrain must not: strip the 8-byte coordinate header and the bodies
    // should differ.
    let addr = start_server();
    let joined = join(addr, "DistinctTester", 0x44);

    let mut bodies: Vec<&[u8]> = joined.chunks.iter().map(|(_, _, p)| &p[8..]).collect();
    let total = bodies.len();
    bodies.sort_unstable();
    bodies.dedup();
    assert!(
        bodies.len() > total / 2,
        "only {} of {total} chunk bodies were unique — terrain looks repetitive",
        bodies.len()
    );
}
