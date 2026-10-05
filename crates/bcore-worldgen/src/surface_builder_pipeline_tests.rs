//! Exercise the active scheduler adapter against complete native surface slabs.
use crate::{biome, ChunkPos, GeneratedChunk, WorldGenerator, MAX_Y, MIN_Y};
use serde_json::Value;

fn number(value: &Value) -> i32 {
    value.as_i64().unwrap().try_into().unwrap()
}

fn decode(columns: &Value) -> Vec<u32> {
    let mut states = vec![0; 384 * 256];
    for (index, palette) in columns["columns"].as_array().unwrap().iter().enumerate() {
        let mut start = MIN_Y;
        for run in columns["palette"][palette.as_u64().unwrap() as usize]
            .as_array()
            .unwrap()
            .chunks_exact(2)
        {
            let end = number(&run[0]);
            for y in start..end {
                states[(y - MIN_Y) as usize * 256 + index] = number(&run[1]) as u32;
            }
            start = end;
        }
        assert_eq!(start, MAX_Y + 1);
    }
    states
}

#[test]
fn active_surface_stage_matches_native_slabs_and_retains_fluid_marks() {
    let reference: Value =
        serde_json::from_str(include_str!("../data/surface_builder_26_1.json")).unwrap();
    let graph = crate::VanillaGraph::load().unwrap();
    let mut checked = 0;
    for sample in reference["samples"].as_array().unwrap() {
        if sample["preliminary_mode"] != "native" || sample["rule"] != "overworld" {
            continue;
        }
        let cx = number(&sample["chunk"][0]);
        let cz = number(&sample["chunk"][1]);
        let mut chunk = GeneratedChunk::new(ChunkPos::new(cx, cz));
        let input = decode(&reference["profiles"][sample["profile"].as_str().unwrap()]);
        for (i, state) in input.into_iter().enumerate() {
            chunk.set(i % 16, MIN_Y + (i / 256) as i32, (i / 16) % 16, state);
        }
        let existing_mark = (3, MIN_Y, 7);
        chunk.postprocessing.push(existing_mark);
        WorldGenerator::new(sample["seed"].as_i64().unwrap()).generate_surface_with_biomes(
            &mut chunk,
            graph,
            |_, qx, qy, qz| {
                assert!((MIN_Y >> 2..=MAX_Y >> 2).contains(&qy));
                let source = sample["biome_source"].as_str().unwrap();
                let name = match source {
                    "overworld" => {
                        let x = qx - cx * 4 + 1;
                        let z = qz - cz * 4 + 1;
                        assert!((0..6).contains(&x) && (0..6).contains(&z));
                        let index = (qy + 16) as usize * 36 + z as usize * 6 + x as usize;
                        let palette = number(&sample["noise_biomes"]["quarts"][index]) as usize;
                        sample["noise_biomes"]["palette"][palette].as_str().unwrap()
                    }
                    "mixed" => {
                        let biomes = reference["mixed_biomes"].as_array().unwrap();
                        biomes[(qx * 31 + qz * 17 + qy).rem_euclid(biomes.len() as i32) as usize]
                            .as_str()
                            .unwrap()
                    }
                    name => name,
                };
                biome::id(name).unwrap()
            },
        );
        let expected = decode(&sample["states"]);
        let mismatch = chunk
            .states()
            .iter()
            .zip(&expected)
            .position(|(a, b)| a != b);
        assert_eq!(
            mismatch, None,
            "{}: first mismatched wire voxel",
            sample["id"]
        );
        assert_eq!(chunk.postprocessing[0], existing_mark);
        let mut added = chunk.postprocessing[1..].to_vec();
        added.sort_by_key(|&(_, y, _)| (y - MIN_Y) >> 4);
        assert_eq!(
            serde_json::json!(added),
            sample["postprocessing"],
            "{}",
            sample["id"]
        );
        for z in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    chunk.surface_y(x, z).map_or(MIN_Y, |y| y + 1),
                    number(&sample["world_surface"][z * 16 + x]),
                    "{} ({x}, {z})",
                    sample["id"]
                );
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 70);
}
