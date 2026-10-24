use super::{catalog, place_named, GenerationEnvironment, Outcome};
use crate::block_predicate::{FeatureEnvironment, RegistryEnvironment};
use crate::dripstone::CaveRandomState;
use crate::feature_sorter::{possible_biomes, sorter};
use crate::feature_world::FeatureError;
use crate::generation::graph::{layer_positions, ChunkPyramid, ChunkStatus};
use crate::region::FeatureRegion;
use crate::simplex::WorldgenRandom;
use crate::{block, ChunkPos, WorldGenerator};
use std::collections::BTreeSet;

#[test]
fn all_overworld_feature_memberships_use_shared_resource_identity() {
    let documents = &catalog().documents;
    let native_ids = documents["biome_ids"].as_object().unwrap();
    let placed = documents["placed_feature"].as_object().unwrap();
    assert_eq!(native_ids.len(), bcore_registry::BIOME_COUNT);
    for biome in possible_biomes() {
        let name = format!("minecraft:{biome}");
        let wire = crate::biome::id(&name).unwrap();
        let native = native_ids[&name].as_u64().unwrap() as u32;
        assert_eq!(wire, native, "shared biome identity {name}");
        let expected: BTreeSet<_> = documents["biome"][&name]["features"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|step| step.as_array().unwrap())
            .map(|feature| feature.as_str().unwrap())
            .collect();
        for feature in placed.keys() {
            let admitted = expected.contains(feature.as_str());
            assert_eq!(
                GenerationEnvironment
                    .biome_has_feature(wire, feature)
                    .unwrap(),
                admitted,
                "{name}, wire {wire}, native {native}, {feature}"
            );
            assert_eq!(
                RegistryEnvironment
                    .biome_has_feature(native, feature)
                    .unwrap(),
                admitted,
                "standalone native identity must remain valid: {name}, {feature}"
            );
        }
    }
}

fn flooded_region(biome: &str) -> FeatureRegion {
    let source = ChunkPos::new(0, 0);
    let mut region = FeatureRegion::shared(WorldGenerator::new(1234));
    for pos in layer_positions(source, 1) {
        let chunk = region.owned_chunk_mut(pos);
        chunk.noise_biomes = Some(vec![crate::biome::id(biome).unwrap(); 1536]);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 62, z, block::DIRT);
                chunk.set(x, 63, z, block::WATER);
                chunk.set(x, 64, z, block::WATER);
            }
        }
    }
    let step = ChunkPyramid::Generation.step(ChunkStatus::Features);
    let available = layer_positions(source, step.direct.radius())
        .map(|pos| ((pos.x, pos.z), step.direct.at(source, pos).unwrap()))
        .collect();
    region.begin_source(source, ChunkStatus::Features, available);
    region
}

#[test]
fn named_seagrass_stream_uses_shared_swamp_identity_in_the_live_region() {
    let name = "seagrass_swamp";
    let wire = crate::biome::id("swamp").unwrap();
    let native = catalog().documents["biome_ids"]["minecraft:swamp"]
        .as_u64()
        .unwrap() as u32;
    assert_eq!(wire, native);
    assert!(RegistryEnvironment.biome_has_feature(native, name).unwrap());
    assert!(RegistryEnvironment.biome_has_feature(wire, name).unwrap());
    for (biome, expected) in [("swamp", true), ("taiga", false)] {
        let mut region = flooded_region(biome);
        let mut random = WorldgenRandom::new(1234);
        let seed = random.set_decoration_seed(1234, 0, 0);
        let (step, index) = sorter().within_step_index(name).unwrap();
        random.set_feature_seed(seed, index as i32, step as i32);
        let result = place_named(
            name,
            &mut region,
            &mut random,
            ChunkPos::new(0, 0),
            &mut CaveRandomState::default(),
        )
        .unwrap();
        assert!(matches!(result, Outcome::Complete(placed) if placed == expected));
        let chunk = region.owned_chunk(ChunkPos::new(0, 0)).unwrap();
        let plants = chunk
            .states()
            .iter()
            .filter(|&&state| {
                catalog().is_block(state, "seagrass").unwrap()
                    || catalog().is_block(state, "tall_seagrass").unwrap()
            })
            .count();
        assert_eq!(plants != 0, expected, "{biome}: {plants} seagrass blocks");
        println!("Named {name} stream in {biome}: {plants} seagrass blocks");
    }
}

#[test]
fn invalid_biomes_remain_errors_and_fresh_environment_uses_native_observations() {
    let native = catalog().documents["biome_ids"].as_object().unwrap();
    for entry in &bcore_registry::vanilla().registries["BIOME"] {
        assert_eq!(native[&entry.key].as_u64(), Some(u64::from(entry.id)));
    }
    assert!(matches!(
        GenerationEnvironment.biome_has_feature(u32::MAX, "seagrass_swamp"),
        Err(FeatureError::MissingData(_))
    ));
    let world = flooded_region("swamp");
    // Native WorldGenRegion at FEATURES has no initialized light storage yet:
    // its sky query is 15 even underwater. Warm swamp water must not freeze.
    assert_eq!(
        GenerationEnvironment
            .raw_brightness(&world, (0, 63, 0))
            .unwrap(),
        15
    );
    assert!(!GenerationEnvironment
        .should_freeze(&world, (0, 63, 0))
        .unwrap());
}
