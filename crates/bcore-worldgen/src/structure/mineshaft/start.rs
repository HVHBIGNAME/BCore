use std::sync::OnceLock;

use super::{MineType, MineshaftLayout};
use crate::structure::placement::{large_feature_random, RandomSpreadPlacement};
use crate::{biome, ChunkPos, MIN_Y, SEA_LEVEL};

struct Settings {
    biomes: [Vec<u32>; 2],
    indices: [i32; 2],
}

fn settings() -> &'static Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../../data/mineshaft_starts_26_1.json"))
                .expect("native mineshaft admission data");
        let names = ["mineshaft", "mineshaft_mesa"];
        Settings {
            biomes: names.map(|name| {
                data[format!("{name}_biomes")]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|name| biome::id(name.as_str().unwrap()).expect("known mineshaft biome"))
                    .collect()
            }),
            indices: names.map(|name| {
                data["structure_steps"][3]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|value| value.as_str() == Some(&format!("minecraft:{name}")))
                    .expect("mineshaft structure index") as i32
            }),
        }
    })
}

impl MineType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "minecraft:mineshaft",
            Self::Mesa => "minecraft:mineshaft_mesa",
        }
    }

    pub fn feature_index(self) -> i32 {
        settings().indices[usize::from(self == Self::Mesa)]
    }
}

impl MineshaftLayout {
    /// Native set frequency, weighted retry order and generation-point biome
    /// admission. The height and biome providers query the pre-structure router.
    pub fn for_chunk(
        seed: i64,
        source: ChunkPos,
        mut biome_at: impl FnMut([i32; 3]) -> u32,
        mut base_height: impl FnMut(i32, i32) -> i32,
    ) -> Option<Self> {
        if !RandomSpreadPlacement::MINESHAFTS.is_candidate(seed, source) {
            return None;
        }
        let mut selection = large_feature_random(seed, source);
        let mut remaining = vec![MineType::Normal, MineType::Mesa];
        while !remaining.is_empty() {
            let selected = selection.next_int(remaining.len());
            let mine_type = remaining.remove(selected);
            let mut random = large_feature_random(seed, source);
            let (layout, point) = Self::start(
                &mut random,
                source,
                mine_type,
                SEA_LEVEL,
                MIN_Y,
                &mut base_height,
            );
            if settings().biomes[usize::from(mine_type == MineType::Mesa)]
                .contains(&biome_at(point))
            {
                return Some(layout);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldGenerator;
    use serde_json::json;

    #[test]
    fn start_admission_and_piece_hashes_match_native_create_structures() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../data/mineshaft_starts_26_1.json")).unwrap();
        for sample in fixture["samples"].as_array().unwrap() {
            let seed = sample["seed"].as_i64().unwrap();
            let source = ChunkPos::new(
                sample["chunk"][0].as_i64().unwrap() as i32,
                sample["chunk"][1].as_i64().unwrap() as i32,
            );
            let biome_source = sample["source"].as_str().unwrap();
            let generator = WorldGenerator::new(seed);
            let mut queries = Vec::new();
            let layout = MineshaftLayout::for_chunk(
                seed,
                source,
                |p| {
                    let id = if biome_source == "overworld" {
                        generator.noise_biome_vanilla(p)
                    } else {
                        biome::id(biome_source).unwrap()
                    };
                    queries.push((p, biome::name(id)));
                    id
                },
                |x, z| generator.base_height_vanilla(x, z),
            );
            let expected = sample["starts"].as_array().unwrap();
            assert_eq!(
                layout.is_some(),
                !expected.is_empty(),
                "seed {seed} {source:?} {biome_source} {queries:?}"
            );
            if let Some(layout) = layout {
                let expected = &expected[0];
                assert_eq!(layout.mine_type.name(), expected["id"].as_str().unwrap());
                assert_eq!(
                    json!(layout.bounds().array()),
                    expected["bounds"],
                    "seed {seed} {source:?} {biome_source}"
                );
                assert_eq!(
                    layout.pieces.len() as u64,
                    expected["pieces"].as_u64().unwrap()
                );
                let mut digest = md5::Context::new();
                for piece in &layout.pieces {
                    digest.consume(piece.save_data(layout.mine_type).to_string().as_bytes());
                }
                assert_eq!(
                    format!("{:x}", digest.compute()),
                    expected["pieces_md5"].as_str().unwrap(),
                    "seed {seed} {source:?} {biome_source}"
                );
            }
        }
        assert_eq!(MineType::Normal.feature_index(), 1);
        assert_eq!(MineType::Mesa.feature_index(), 2);
    }

    #[test]
    fn base_heights_match_native_noise_columns() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../data/mineshafts_26_1.json")).unwrap();
        for sample in fixture["samples"].as_array().unwrap() {
            let seed = sample["seed"].as_i64().unwrap();
            let p = &sample["base_height_position"];
            let (x, z) = (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32);
            assert_eq!(
                WorldGenerator::new(seed).base_height_vanilla(x, z),
                sample["base_height"].as_i64().unwrap() as i32,
                "seed {seed} ({x}, {z})"
            );
        }
    }
}
