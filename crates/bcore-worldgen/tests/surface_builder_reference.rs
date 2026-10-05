// Compile the builder against public types as well as the library export.
pub use bcore_worldgen::{
    biome, biome_zoom, block, density, is_air, noise_perlin, simplex, surface, surface_rules,
    GeneratedChunk, MAX_Y, MIN_Y, SEA_LEVEL,
};
#[path = "../src/surface_builder.rs"]
mod surface_builder;

use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Columns {
    palette: Vec<Vec<i32>>,
    columns: Vec<usize>,
}

impl Columns {
    fn decode(&self) -> Vec<u32> {
        assert_eq!(self.columns.len(), 256);
        let mut states = vec![0; 384 * 256];
        for (i, &palette) in self.columns.iter().enumerate() {
            let runs = &self.palette[palette];
            assert_eq!(runs.len() % 2, 0);
            let mut start = MIN_Y;
            for run in runs.chunks_exact(2) {
                let [end, state] = [run[0], run[1]];
                assert!(end > start && end <= MAX_Y + 1 && state >= 0);
                for y in start..end {
                    states[(y - MIN_Y) as usize * 256 + i] = state as u32;
                }
                start = end;
            }
            assert_eq!(start, MAX_Y + 1);
        }
        states
    }
}

#[derive(Deserialize)]
struct NoiseBiomes {
    palette: Vec<String>,
    quarts: Vec<usize>,
}

#[derive(Deserialize)]
struct Sample {
    id: String,
    seed: i64,
    chunk: [i32; 2],
    biome_source: String,
    profile: String,
    rule: String,
    preliminary_mode: String,
    preliminary_corners: [i32; 4],
    states: Columns,
    states_sha256: String,
    rule_context_sha256: String,
    rule_calls: usize,
    postprocessing: Vec<(usize, i32, usize)>,
    changed_to: BTreeMap<u32, usize>,
    surface_depths: Vec<i32>,
    surface_secondary_bits: Vec<String>,
    world_surface: Vec<i32>,
    noise_biomes: Option<NoiseBiomes>,
}

impl Sample {
    fn biome_at(&self, reference: &Reference, qx: i32, qy: i32, qz: i32) -> biome::BiomeId {
        assert!(
            (MIN_Y >> 2..=MAX_Y >> 2).contains(&qy),
            "{}: unclamped quart Y {qy}",
            self.id
        );
        let name = match self.biome_source.as_str() {
            "mixed" => {
                &reference.mixed_biomes[(qx * 31 + qz * 17 + qy)
                    .rem_euclid(reference.mixed_biomes.len() as i32)
                    as usize]
            }
            "overworld" => {
                let biomes = self.noise_biomes.as_ref().unwrap();
                let x = qx - self.chunk[0] * 4 + 1;
                let z = qz - self.chunk[1] * 4 + 1;
                assert!((0..6).contains(&x) && (0..6).contains(&z));
                let i = (qy + 16) as usize * 36 + z as usize * 6 + x as usize;
                &biomes.palette[biomes.quarts[i]]
            }
            name => name,
        };
        biome::id(name).unwrap()
    }
}

#[derive(Deserialize)]
struct Reference {
    minecraft: String,
    jar_sha256: String,
    probe_sha256: String,
    profiles: HashMap<String, Columns>,
    rules: HashMap<String, Value>,
    mixed_biomes: Vec<String>,
    samples: Vec<Sample>,
}

fn reference() -> &'static Reference {
    static REFERENCE: OnceLock<Reference> = OnceLock::new();
    REFERENCE.get_or_init(|| {
        let result: Reference =
            serde_json::from_str(include_str!("../data/surface_builder_26_1.json")).unwrap();
        assert_eq!(result.minecraft, "26.1");
        assert_eq!(result.samples.len(), 137);
        let predicates: Value =
            serde_json::from_str(include_str!("../data/surface_builder_blocks_26_1.json")).unwrap();
        assert_eq!(predicates["jar_sha256"], result.jar_sha256);
        assert_eq!(predicates["probe_sha256"], result.probe_sha256);
        let bundle: Value =
            serde_json::from_str(include_str!("../data/vanilla_worldgen.json")).unwrap();
        assert_eq!(
            bundle["worldgen_version.json"]["jar_sha256"],
            result.jar_sha256
        );
        assert_eq!(
            bundle["noise_settings/overworld.json"]["surface_rule"],
            result.rules["overworld"]
        );
        result
    })
}

fn initial_chunk(sample: &Sample, input: &[u32]) -> GeneratedChunk {
    static TEMPLATE: OnceLock<GeneratedChunk> = OnceLock::new();
    let mut chunk = TEMPLATE
        .get_or_init(|| WorldGenerator::new(0).generate_chunk(ChunkPos::new(0, 0)))
        .clone();
    chunk.pos = ChunkPos::new(sample.chunk[0], sample.chunk[1]);
    for (i, &state) in input.iter().enumerate() {
        chunk.set(i % 16, MIN_Y + (i / 256) as i32, (i / 16) % 16, state);
    }
    chunk
}

fn check_sample(sample: &Sample, reference: &Reference) {
    density::clear_density_caches();
    let rule = surface_rules::SurfaceRule::parse(&reference.rules[&sample.rule]);
    let input = reference.profiles[&sample.profile].decode();
    let expected = sample.states.decode();
    let mut chunk = initial_chunk(sample, &input);
    let preliminary = density::parse_router("overworld", "preliminary_surface_level").unwrap();
    let ctx = density::EvalContext {
        seed: sample.seed,
        ..Default::default()
    }
    .with_noise_bounds(sample.chunk[0] * 16, sample.chunk[1] * 16, 4);
    let mut corner_calls = Vec::new();
    let mut context_digest = Sha256::new();
    let mut rule_calls = 0;
    let mut posts = surface_builder::build_surface_observed(
        &mut chunk,
        sample.seed,
        &rule,
        density::noise_registry(),
        |x, z| {
            corner_calls.push([x, z]);
            let i =
                ((x - sample.chunk[0] * 16) / 16 + (z - sample.chunk[1] * 16) / 16 * 2) as usize;
            let height = if sample.preliminary_mode == "native" {
                density::evaluate(&preliminary, x as f64, 0.0, z as f64, &ctx).floor() as i32
            } else {
                assert_eq!(sample.preliminary_mode, "prescribed");
                sample.preliminary_corners[i]
            };
            assert_eq!(
                height, sample.preliminary_corners[i],
                "{}: preliminary ({x}, {z})",
                sample.id
            );
            height
        },
        |_, qx, qy, qz| sample.biome_at(reference, qx, qy, qz),
        |context, result| {
            for value in [
                context.x,
                context.y,
                context.z,
                context.stone_depth_above,
                context.stone_depth_below,
                context.water_height,
                context.surface_depth,
                context.preliminary_surface_level + context.surface_depth - 8,
                result.map_or(-1, |state| state as i32),
            ] {
                context_digest.update(value.to_le_bytes());
            }
            rule_calls += 1;
        },
    );
    assert_eq!(corner_calls.len(), 4, "{}", sample.id);
    let mismatches: Vec<_> = chunk
        .states()
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .take(12)
        .map(|(i, (&actual, &expected))| {
            (
                i % 16,
                MIN_Y + (i / 256) as i32,
                (i / 16) % 16,
                actual,
                expected,
            )
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "{}: (x, y, z, actual, native) {mismatches:?}",
        sample.id
    );
    let mut states_digest = Sha256::new();
    let mut changed = BTreeMap::new();
    for (i, &state) in chunk.states().iter().enumerate() {
        states_digest.update(state.to_le_bytes());
        if state != input[i] {
            *changed.entry(state).or_insert(0) += 1;
        }
    }
    assert_eq!(
        format!("{:x}", states_digest.finalize()),
        sample.states_sha256,
        "{}",
        sample.id
    );
    assert_eq!(changed, sample.changed_to, "{}", sample.id);
    assert_eq!(
        rule_calls, sample.rule_calls,
        "{}: default-block rule visits",
        sample.id
    );
    assert_eq!(
        format!("{:x}", context_digest.finalize()),
        sample.rule_context_sha256,
        "{}: whole native scan contexts (stone depths, water, min surface, rule result)",
        sample.id
    );
    // Native storage groups marks by section while retaining insertion order within it.
    posts.sort_by_key(|&(_, y, _)| (y - MIN_Y) >> 4);
    assert_eq!(
        posts, sample.postprocessing,
        "{}: fluid postprocessing",
        sample.id
    );
    for z in 0..16 {
        for x in 0..16 {
            let i = z * 16 + x;
            let wx = sample.chunk[0] * 16 + x as i32;
            let wz = sample.chunk[1] * 16 + z as i32;
            assert_eq!(
                surface_rules::surface_depth_with_noise(
                    density::noise_registry(),
                    sample.seed,
                    wx,
                    wz
                ),
                sample.surface_depths[i],
                "{} ({x},{z}): depth",
                sample.id
            );
            let secondary = density::noise_registry().sample(
                "minecraft:surface_secondary",
                sample.seed,
                wx as f64,
                0.0,
                wz as f64,
            );
            assert_eq!(
                format!("{:016x}", secondary.to_bits()),
                sample.surface_secondary_bits[i],
                "{} ({x},{z}): secondary",
                sample.id
            );
            assert_eq!(
                chunk.surface_y(x, z).map_or(MIN_Y, |y| y + 1),
                sample.world_surface[i],
                "{} ({x},{z}): heightmap",
                sample.id
            );
        }
    }
    density::clear_density_caches();
}

#[test]
fn complete_surface_slabs_and_scan_contexts_match_native_26_1() {
    let reference = reference();
    for sample in &reference.samples {
        check_sample(sample, reference);
    }
}

#[test]
fn extension_and_scan_witnesses_are_non_vacuous() {
    let reference = reference();
    let changed = |prefix: &str, state: u32| {
        reference
            .samples
            .iter()
            .filter(|s| s.id.starts_with(prefix))
            .map(|s| s.changed_to.get(&state).copied().unwrap_or(0))
            .sum::<usize>()
    };
    assert!(changed("iceberg/", 12914) > 1000, "packed ice extension");
    assert!(
        changed("iceberg/", block::SNOW_BLOCK) > 100,
        "iceberg snow caps"
    );
    assert!(changed("badlands/", 12912) > 1000, "terracotta bands");
    assert!(changed("scan/depth", 24687) > 100, "cavity ceilings");
    assert!(
        changed("scan/fluid_writes", block::WATER) > 100,
        "rule-created fluids"
    );
    assert!(changed("scan/temperature", 6927) > 100, "cold surface rule");
    assert!(reference
        .samples
        .iter()
        .filter(|s| s.id.starts_with("scan/"))
        .any(|s| !s.postprocessing.is_empty()));
    assert!(
        reference
            .samples
            .iter()
            .flat_map(|s| &s.surface_depths)
            .any(|&d| d <= 0),
        "surface holes"
    );
}

#[test]
fn public_builder_is_repeatable_across_seed_and_chunk_order() {
    let reference = reference();
    let probes: Vec<_> = reference
        .samples
        .iter()
        .filter(|s| {
            s.id.starts_with("iceberg/")
                || s.id.starts_with("badlands/")
                || s.id.starts_with("steep/")
        })
        .collect();
    for sample in probes.into_iter().rev().step_by(3) {
        check_sample(sample, reference);
    }
    // The public entry point uses the same pass with observation compiled away.
    let sample = reference
        .samples
        .iter()
        .find(|s| s.id == "scan/depth")
        .unwrap();
    let mut chunk = initial_chunk(sample, &reference.profiles[&sample.profile].decode());
    let mut standalone = chunk.clone();
    let rule = surface_rules::SurfaceRule::parse(&reference.rules[&sample.rule]);
    let posts = bcore_worldgen::surface_builder::build_surface(
        &mut chunk,
        sample.seed,
        &rule,
        density::noise_registry(),
        |x, z| {
            sample.preliminary_corners
                [((x - sample.chunk[0] * 16) / 16 + (z - sample.chunk[1] * 16) / 16 * 2) as usize]
        },
        |_, qx, qy, qz| sample.biome_at(reference, qx, qy, qz),
    );
    assert!(posts.is_empty());
    assert_eq!(chunk.states(), sample.states.decode());
    let standalone_posts = surface_builder::build_surface(
        &mut standalone,
        sample.seed,
        &rule,
        density::noise_registry(),
        |x, z| {
            sample.preliminary_corners
                [((x - sample.chunk[0] * 16) / 16 + (z - sample.chunk[1] * 16) / 16 * 2) as usize]
        },
        |_, qx, qy, qz| sample.biome_at(reference, qx, qy, qz),
    );
    assert_eq!(standalone_posts, posts);
    assert_eq!(standalone.states(), chunk.states());
}
