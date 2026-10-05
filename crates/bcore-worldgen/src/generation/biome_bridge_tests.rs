use super::{catalog, place_named, GenerationEnvironment, Outcome};
use crate::block_predicate::{FeatureEnvironment, RegistryEnvironment};
use crate::dripstone::CaveRandomState;
use crate::feature_sorter::{sorter, POSSIBLE_BIOMES};
use crate::feature_world::FeatureError;
use crate::generation::graph::{layer_positions, ChunkPyramid, ChunkStatus};
use crate::region::FeatureRegion;
use crate::simplex::WorldgenRandom;
use crate::{block, ChunkPos, WorldGenerator};
use std::collections::BTreeSet;

#[test]
fn all_overworld_feature_memberships_bridge_by_resource_identity() {
    let documents = &catalog().documents;
    let native_ids = documents["biome_ids"].as_object().unwrap();
    let placed = documents["placed_feature"].as_object().unwrap();
    let wire = crate::assets::load("biome_registry.json").unwrap();
    assert_eq!(native_ids.len(), 65);
    assert_eq!(wire.as_array().unwrap().len(), 66);
    let mut shifted = Vec::new();
    let (mut checked, mut false_rejections, mut false_admissions) = (0, 0, 0);
    for biome in POSSIBLE_BIOMES {
        let name = format!("minecraft:{biome}");
        let wire = crate::biome::id(&name).unwrap();
        let native = native_ids[&name].as_u64().unwrap() as u32;
        if wire != native {
            shifted.push(format!("{biome}: {wire}->{native}"));
        }
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
            if let Ok(unbridged) = RegistryEnvironment.biome_has_feature(wire, feature) {
                false_rejections += usize::from(admitted && !unbridged);
                false_admissions += usize::from(!admitted && unbridged);
            }
            checked += 1;
        }
    }
    assert!(!shifted.is_empty());
    assert!(false_rejections > 0 && false_admissions > 0);
    println!(
        "Biome bridge: {} possible biomes, {} placed definitions, {checked} membership checks, {} shifted overworld IDs; old numeric path falsely rejected {false_rejections} and admitted {false_admissions} memberships",
        POSSIBLE_BIOMES.len(), placed.len(), shifted.len()
    );
    println!("Shifted identities: {}", shifted.join(", "));
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
fn named_seagrass_stream_uses_shifted_swamp_identity_in_the_live_region() {
    let name = "seagrass_swamp";
    let wire = crate::biome::id("swamp").unwrap();
    let native = catalog().documents["biome_ids"]["minecraft:swamp"]
        .as_u64()
        .unwrap() as u32;
    assert_ne!(wire, native);
    assert!(RegistryEnvironment.biome_has_feature(native, name).unwrap());
    assert!(!RegistryEnvironment.biome_has_feature(wire, name).unwrap());
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
fn unmapped_biomes_remain_errors_and_fresh_environment_uses_native_observations() {
    let native = catalog().documents["biome_ids"].as_object().unwrap();
    let wire = crate::assets::load("biome_registry.json").unwrap();
    let extra: Vec<_> = wire
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, name)| !native.contains_key(name.as_str().unwrap()))
        .collect();
    assert_eq!(extra.len(), 1);
    for (id, name) in extra {
        assert!(matches!(
            GenerationEnvironment.biome_has_feature(id as u32, "seagrass_swamp"),
            Err(FeatureError::MissingData(_))
        ));
        println!("Unmapped biome remains explicit: {name}, wire ID {id}");
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
