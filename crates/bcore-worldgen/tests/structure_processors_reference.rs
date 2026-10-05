use bcore_worldgen::feature_world::{FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::simplex::JavaRandom;
use bcore_worldgen::structure::processors::{process_block_infos, Processor};
use bcore_worldgen::structure::template::{BlockInfo, Nbt, PlacementSettings, ProcessorRandom};
use bcore_worldgen::structure::template_pool::StructureAssets;
use serde_json::{json, Value};

struct NoWorldReads;
impl OreWorld for NoWorldReads {
    fn ocean_floor_wg(&self, _: i32, _: i32) -> i32 {
        panic!("pure processor height read")
    }
    fn get_block(&self, _: Pos) -> Option<u32> {
        panic!("pure processor world read")
    }
    fn set_block(&mut self, _: Pos, _: u32) -> bool {
        panic!("pure processor write")
    }
}
impl FeatureWorld for NoWorldReads {
    fn feature_biome(&self, _: Pos) -> u32 {
        panic!("pure processor biome read")
    }
    fn feature_height(&self, _: FeatureHeightmap, _: i32, _: i32) -> i32 {
        panic!("pure processor height read")
    }
    fn can_write_feature(&self, _: Pos) -> bool {
        panic!("pure processor write check")
    }
    fn set_feature_block(&mut self, _: Pos, _: u32, _: i32) -> bool {
        panic!("pure processor block write")
    }
    fn mark_feature_postprocessing(&mut self, _: Pos) {
        panic!("pure processor mark")
    }
    fn schedule_feature_tick(&mut self, _: bcore_worldgen::tick_request::TickRequest) -> bool {
        panic!("pure processor tick")
    }
}

#[test]
fn native_structure_processors_match_every_state_and_rng_continuation() {
    let fixture: Value =
        serde_json::from_str(include_str!("../data/structure_processors_extra_26_1.json")).unwrap();
    let assets = StructureAssets::bundled();
    assert_eq!(fixture["jar_sha256"], StructureAssets::JAR_SHA256);
    let mut calls = 0;
    for row in fixture["cases"].as_array().unwrap() {
        let operation = json!({"processor_type":format!("minecraft:{}",row["processor"].as_str().unwrap()),"mossiness":row["mossiness"]});
        let kept: Nbt = serde_json::from_value(row["kept"]["nbt"].clone()).unwrap();
        let inputs: Vec<_> = (0..assets.blocks.len())
            .map(|i| BlockInfo {
                pos: ((i % 37) as i32, (i % 91) as i32, ((i / 37) % 41) as i32),
                state: i as u32,
                nbt: Some(kept.clone()),
            })
            .collect();
        let settings = PlacementSettings {
            processors: vec![
                Processor::from_json(&operation, &assets.blocks, &assets.tags).unwrap(),
            ],
            ..Default::default()
        };
        let mut random = JavaRandom::new(row["seed"].as_i64().unwrap());
        let mut settings_random = ProcessorRandom(if row["supplied_random"] == true {
            Some(&mut random)
        } else {
            None
        });
        let actual = process_block_infos(
            &NoWorldReads,
            &assets.blocks,
            (-18, -47, -20),
            (7, 2, -9),
            &inputs,
            &settings,
            &mut settings_random,
        )
        .unwrap();
        let expected = row["states"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for (i, (result, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                json!(result.state),
                *expected,
                "{operation}, supplied={}, state={i}",
                row["supplied_random"]
            );
            assert_eq!(
                result.pos,
                (
                    inputs[i].pos.0 - 18,
                    inputs[i].pos.1 - 47,
                    inputs[i].pos.2 - 20
                )
            );
            assert_eq!(result.nbt, Some(kept.clone()));
        }
        assert_eq!(
            random.next_long(),
            row["next_i64"].as_i64().unwrap(),
            "{operation} RNG"
        );
        calls += actual.len();
    }
    assert_eq!(calls, 328603);
}
