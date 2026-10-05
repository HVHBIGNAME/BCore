//! Isolate observed placed-feature discrepancies with complete native input chunks.
//! Uses production placement/base/ore kernels; varies only the heightmap adapter.
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use bcore_worldgen::base_features::BaseFeatures;
use bcore_worldgen::biome_zoom::BiomeZoom;
use bcore_worldgen::block_predicate::{catalog, RegistryEnvironment};
use bcore_worldgen::feature_world::{FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::simplex::{WorldgenRandom, Xoroshiro128};
use bcore_worldgen::tick_request::TickRequest;
use serde_json::{json, Value};

struct Chunk {
    states: Vec<u32>,
    biomes: Vec<u32>,
    heights: BTreeMap<String, Vec<i32>>,
}

struct NativeWorld {
    chunks: BTreeMap<(i32, i32), Chunk>,
    zoom: BiomeZoom,
    source: (i32, i32),
    retained: bool,
    retained_final: bool,
    writes: Vec<[i32; 5]>,
}

impl NativeWorld {
    fn new(input: &Value, retained: bool, retained_final: bool) -> Result<Self, Box<dyn Error>> {
        let seed: i64 = input["seed"].as_str().ok_or("seed")?.parse()?;
        let mut chunks = BTreeMap::new();
        for value in input["before"].as_array().ok_or("before")? {
            if value["min_y"] != -64 || value["states"].as_array().ok_or("states")?.len() != 98304 {
                return Err("this diagnostic requires complete fresh-overworld chunks".into());
            }
            if value["is_light_on"] == true {
                return Err(
                    "initialized light requires a different native environment fixture".into(),
                );
            }
            let x = value["pos"][0].as_i64().ok_or("chunk x")? as i32;
            let z = value["pos"][1].as_i64().ok_or("chunk z")? as i32;
            let states = value["states"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect();
            let biomes = value["biomes"]
                .as_array()
                .ok_or("biomes")?
                .iter()
                .map(|v| {
                    catalog().documents["biome_ids"][v.as_str().unwrap()]
                        .as_u64()
                        .expect("native catalog biome") as u32
                })
                .collect();
            let heights = value["heightmaps"]
                .as_object()
                .ok_or("heightmaps")?
                .iter()
                .map(|(name, values)| {
                    (
                        name.clone(),
                        values
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_i64().unwrap() as i32)
                            .collect(),
                    )
                })
                .collect();
            chunks.insert(
                (x, z),
                Chunk {
                    states,
                    biomes,
                    heights,
                },
            );
        }
        let source = (
            input["source"][0].as_i64().ok_or("source x")? as i32,
            input["source"][1].as_i64().ok_or("source z")? as i32,
        );
        Ok(Self {
            chunks,
            zoom: BiomeZoom::new(seed),
            source,
            retained,
            retained_final,
            writes: Vec::new(),
        })
    }

    fn index((x, y, z): Pos) -> usize {
        ((y + 64) * 256 + (z & 15) * 16 + (x & 15)) as usize
    }

    fn matches_height(kind: FeatureHeightmap, state: u32) -> bool {
        let info = catalog().info(state).unwrap();
        match kind {
            FeatureHeightmap::WorldSurface | FeatureHeightmap::WorldSurfaceWg => !info.is_air(),
            FeatureHeightmap::OceanFloor | FeatureHeightmap::OceanFloorWg => info.blocks_motion(),
            _ => panic!("heightmap outside this vegetation diagnostic"),
        }
    }
}

impl OreWorld for NativeWorld {
    fn get_block(&self, pos: Pos) -> Option<u32> {
        if !(-64..320).contains(&pos.1) {
            return Some(0);
        }
        Some(
            self.chunks
                .get(&(pos.0 >> 4, pos.2 >> 4))
                .expect("read outside captured dependency chunks")
                .states[Self::index(pos)],
        )
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        self.set_feature_block(pos, state, 2)
    }
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.feature_height(FeatureHeightmap::OceanFloorWg, x, z)
    }
}

impl FeatureWorld for NativeWorld {
    fn feature_biome(&self, pos: Pos) -> u32 {
        let (x, y, z) = self.zoom.quart_at(pos);
        let chunk = self
            .chunks
            .get(&(x >> 2, z >> 2))
            .expect("biome outside native captured palette");
        chunk.biomes[((y.clamp(-16, 79) + 16) * 16 + (z & 3) * 4 + (x & 3)) as usize]
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        let (key, wg) = match kind {
            FeatureHeightmap::WorldSurfaceWg => ("WORLD_SURFACE_WG", true),
            FeatureHeightmap::OceanFloorWg => ("OCEAN_FLOOR_WG", true),
            FeatureHeightmap::WorldSurface => ("WORLD_SURFACE", false),
            FeatureHeightmap::OceanFloor => ("OCEAN_FLOOR", false),
            _ => panic!("heightmap outside this vegetation diagnostic"),
        };
        if (self.retained && wg) || (self.retained_final && !wg) {
            if let Some(heights) = self.chunks[&(x >> 4, z >> 4)].heights.get(key) {
                return heights[((z & 15) * 16 + (x & 15)) as usize];
            }
        }
        for y in (-64..320).rev() {
            if Self::matches_height(kind, self.get_block((x, y, z)).unwrap()) {
                return y + 1;
            }
        }
        -64
    }
    fn can_write_feature(&self, (x, y, z): Pos) -> bool {
        (-64..320).contains(&y)
            && (x >> 4).abs_diff(self.source.0) <= 1
            && (z >> 4).abs_diff(self.source.1) <= 1
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        if !self.can_write_feature(pos) {
            return false;
        }
        self.chunks
            .get_mut(&(pos.0 >> 4, pos.2 >> 4))
            .expect("write outside captured region")
            .states[Self::index(pos)] = state;
        self.writes.push([pos.0, pos.1, pos.2, state as i32, flags]);
        let index = ((pos.2 & 15) * 16 + (pos.0 & 15)) as usize;
        for (key, kind) in [
            ("WORLD_SURFACE", FeatureHeightmap::WorldSurface),
            ("OCEAN_FLOOR", FeatureHeightmap::OceanFloor),
        ] {
            let Some(height) = self.chunks[&(pos.0 >> 4, pos.2 >> 4)]
                .heights
                .get(key)
                .map(|v| v[index])
            else {
                continue;
            };
            let next = if Self::matches_height(kind, state) {
                height.max(pos.1 + 1)
            } else if pos.1 == height - 1 {
                (-64..pos.1)
                    .rev()
                    .find(|&y| {
                        Self::matches_height(kind, self.get_block((pos.0, y, pos.2)).unwrap())
                    })
                    .map_or(-64, |y| y + 1)
            } else {
                height
            };
            self.chunks
                .get_mut(&(pos.0 >> 4, pos.2 >> 4))
                .unwrap()
                .heights
                .get_mut(key)
                .unwrap()[index] = next;
        }
        true
    }
    fn mark_feature_postprocessing(&mut self, _: Pos) {
        panic!("unexpected forest-grass postprocessing effect");
    }
    fn schedule_feature_tick(&mut self, _: TickRequest) -> bool {
        panic!("unexpected forest-grass tick effect");
    }
}

fn run(input: &Value, retained: bool, retained_final: bool) -> Result<Value, Box<dyn Error>> {
    let mut world = NativeWorld::new(input, retained, retained_final)?;
    let rng = &input["entry"]["rng"];
    let mut random = WorldgenRandom::new(0);
    random.source = Xoroshiro128::from_state(
        rng["seed_lo"].as_i64().ok_or("seed_lo")? as u64,
        rng["seed_hi"].as_i64().ok_or("seed_hi")? as u64,
    );
    let name = input["feature"].as_str().ok_or("feature")?;
    let ore = name.starts_with("minecraft:ore_");
    let origin = (world.source.0 * 16, -64, world.source.1 * 16);
    let placed = if ore {
        let source = bcore_core::ChunkPos::new(world.source.0, world.source.1);
        bcore_worldgen::features::place_ore_feature(
            &mut world,
            &mut random,
            source,
            name,
            |world, pos| {
                let id = world.feature_biome(pos);
                let biomes = catalog().documents["biome_ids"]
                    .as_object()
                    .expect("native biome identities");
                let biome = biomes
                    .iter()
                    .find(|(_, value)| value.as_u64() == Some(u64::from(id)))
                    .map(|(name, _)| name.strip_prefix("minecraft:").unwrap_or(name))
                    .expect("native biome name");
                bcore_worldgen::feature_sorter::sorter()
                    .feature_in_biome(biome, name.strip_prefix("minecraft:").unwrap())
            },
        )
        .ok_or("ore not implemented by production dispatch")?
    } else {
        if name != "minecraft:patch_grass_forest" && name != "minecraft:seagrass_river" {
            return Err(
                "diagnostic supports production ores, forest-grass and river seagrass".into(),
            );
        }
        bcore_worldgen::placement::place_named(
            name,
            &mut world,
            &mut random,
            origin,
            &RegistryEnvironment,
            &mut BaseFeatures::default(),
        )?
    };
    let mut mismatches = 0;
    let mut examples = Vec::new();
    for value in input["after"].as_array().ok_or("after")? {
        let x = value["pos"][0].as_i64().unwrap() as i32;
        let z = value["pos"][1].as_i64().unwrap() as i32;
        for (i, expected) in value["states"].as_array().unwrap().iter().enumerate() {
            let actual = world.chunks[&(x, z)].states[i];
            if expected.as_u64().unwrap() as u32 != actual {
                mismatches += 1;
                if examples.len() < 16 {
                    examples.push(json!({"pos": [x * 16 + (i % 16) as i32, -64 + (i / 256) as i32, z * 16 + (i / 16 % 16) as i32],
                                         "expected": expected, "actual": actual}));
                }
            }
        }
    }
    let continuation = random.next_long();
    Ok(
        json!({"height_adapter": if retained_final { "native_all_maps" } else if retained { "native_retained_WG" } else { "live_rescan" },
              "mismatches": mismatches, "examples": examples, "placed": placed,
              "placed_matches": Value::Bool(placed) == input["exit"]["result"],
              "next_i64": continuation, "rng_continuation_matches": Value::from(continuation) == input["exit"]["rng"]["next_i64_from_copy"],
              "writes": world.writes, "write_order_scored": !ore,
              "write_order_matches": if ore { None } else { Some(serde_json::to_value(&world.writes)? == input["writes"]) }}),
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: native-feature-replay <case.json> <new-output.json>".into());
    }
    let input: Value = serde_json::from_reader(BufReader::new(File::open(&args[1])?))?;
    let result = json!({"native_provenance": input["native_provenance"], "feature": input["feature"],
                       "variants": [run(&input, true, false)?, run(&input, false, false)?, run(&input, true, true)?]});
    let out = File::options()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    let mut out = BufWriter::new(out);
    serde_json::to_writer_pretty(&mut out, &result)?;
    out.flush()?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
