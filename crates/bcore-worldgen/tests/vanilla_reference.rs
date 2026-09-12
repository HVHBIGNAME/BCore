//! Reference values captured from an independent vanilla 26.1 server.
use bcore_core::ChunkPos;
use bcore_worldgen::{biome, GeneratedChunk, WorldGenerator};
use std::collections::HashMap;

#[test]
fn captured_terrain_and_three_dimensional_biomes_match_vanilla() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("../data/parity_26_1.json")).unwrap();
    let generator = WorldGenerator::new(reference["seed"].as_i64().unwrap());
    let mut chunks: HashMap<(i32, i32), GeneratedChunk> = HashMap::new();
    for sample in reference["samples"].as_array().unwrap() {
        for row in sample["columns"].as_array().unwrap() {
            let x = row[0].as_i64().unwrap() as i32;
            let z = row[1].as_i64().unwrap() as i32;
            let chunk = chunks
                .entry((x.div_euclid(16), z.div_euclid(16)))
                .or_insert_with_key(|&(cx, cz)| {
                    generator.generate_chunk_vanilla(ChunkPos::new(cx, cz))
                });
            assert_eq!(
                chunk.height_at(x.rem_euclid(16) as usize, z.rem_euclid(16) as usize),
                row[3].as_i64().unwrap() as i32,
                "terrain at ({x},{z})"
            );
        }
        for row in sample["biomes"].as_array().unwrap() {
            let x = row[0].as_i64().unwrap() as i32;
            let y = row[1].as_i64().unwrap() as i32;
            let z = row[2].as_i64().unwrap() as i32;
            let chunk = &chunks[&(x.div_euclid(16), z.div_euclid(16))];
            let actual =
                chunk.noise_biome_at(x.rem_euclid(16) as usize, y, z.rem_euclid(16) as usize);
            assert_eq!(
                biome::name(actual),
                row[3].as_str().unwrap().strip_prefix("minecraft:").unwrap(),
                "biome at ({x},{y},{z})"
            );
        }
    }
}
