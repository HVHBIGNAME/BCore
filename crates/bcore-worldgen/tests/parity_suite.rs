//! Optional integration point for the Python parity suite.
//!
//! Set `PARITY_DUMP_REQUESTS` to a file containing `seed cx cz` triples and
//! `PARITY_DUMP_OUT` to receive one whitespace-separated state dump per line.
//! The test is intentionally a no-op otherwise, so normal CI does not need a
//! vanilla server or a generated fixture.
use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;
use std::env;
use std::fs;
use std::io::Write;

fn dump_requested_chunks() {
    let Some(requests) = env::var_os("PARITY_DUMP_REQUESTS") else {
        return;
    };
    let Some(output) = env::var_os("PARITY_DUMP_OUT") else {
        return;
    };
    let request_text = fs::read_to_string(requests).expect("read parity requests");
    let mut file = fs::File::create(output).expect("create parity dump");
    for line in request_text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3 {
            continue;
        }
        let seed: i64 = fields[0].parse().expect("seed");
        let cx: i32 = fields[1].parse().expect("chunk x");
        let cz: i32 = fields[2].parse().expect("chunk z");
        let chunk = WorldGenerator::new(seed).generate_chunk_vanilla(ChunkPos::new(cx, cz));
        write!(file, "{} {}", cx, cz).expect("write parity header");
        for state in chunk.states() {
            write!(file, " {}", state).expect("write parity state");
        }
        writeln!(file).expect("write parity newline");
    }
}

#[test]
fn dump_requested_chunks_machine_readable() {
    dump_requested_chunks();
}

/// Compare a fixture made by `scripts/parity_suite.py` when a CI job provides
/// one. The fixture format is one line per chunk: `cx cz state...`.
#[test]
#[ignore = "requires a vanilla fixture; run scripts/parity_suite.py first"]
fn parity_fixture_matches_bcore() {
    let path = env::var("PARITY_VANILLA_FIXTURE").expect("PARITY_VANILLA_FIXTURE");
    let seed: i64 = env::var("PARITY_SEED")
        .expect("PARITY_SEED")
        .parse()
        .expect("seed");
    let text = fs::read_to_string(path).expect("read parity fixture");
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let cx: i32 = it.next().expect("cx").parse().expect("cx");
        let cz: i32 = it.next().expect("cz").parse().expect("cz");
        let expected: Vec<u32> = it.map(|v| v.parse().expect("state id")).collect();
        let actual = WorldGenerator::new(seed).generate_chunk_vanilla(ChunkPos::new(cx, cz));
        assert_eq!(
            expected.len(),
            actual.states().len(),
            "chunk ({cx},{cz}) length"
        );
        assert_eq!(expected, actual.states(), "chunk ({cx},{cz})");
    }
}
