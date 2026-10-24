use super::{check_lake_config, BaseFeatures};
use crate::block_predicate::{FeatureEnvironment, FeatureResult, RegistryEnvironment};
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::ore::OreWorld;
use crate::placement::ConfiguredFeatureDispatcher;
use crate::simplex::WorldgenRandom;
use crate::tick_request::TickRequest;
use serde_json::{json, Value};

#[derive(Default)]
struct World {
    denied: bool,
    writes: Vec<(Pos, u32, i32)>,
    biome: Option<u32>,
    biome_reads: std::cell::RefCell<Vec<Pos>>,
}

impl OreWorld for World {
    fn ocean_floor_wg(&self, _x: i32, _z: i32) -> i32 {
        crate::MIN_Y
    }
    fn get_block(&self, _pos: Pos) -> Option<u32> {
        Some(crate::block::AIR)
    }
    fn set_block(&mut self, _pos: Pos, _state: u32) -> bool {
        panic!("configured feature must preserve write flags")
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, pos: Pos) -> u32 {
        self.biome_reads.borrow_mut().push(pos);
        self.biome.unwrap_or(crate::biome::DEFAULT_BIOME)
    }
    fn feature_height(&self, _kind: FeatureHeightmap, _x: i32, _z: i32) -> i32 {
        crate::MIN_Y
    }
    fn can_write_feature(&self, _pos: Pos) -> bool {
        !self.denied
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        self.writes.push((pos, state, flags));
        true
    }
    fn mark_feature_postprocessing(&mut self, _pos: Pos) {
        panic!("unexpected postprocessing request")
    }
    fn schedule_feature_tick(&mut self, _request: TickRequest) -> bool {
        panic!("unexpected tick request")
    }
}

#[derive(Default)]
struct Terminal {
    calls: Vec<(Option<String>, Pos, i64)>,
}

impl ConfiguredFeatureDispatcher for Terminal {
    fn place_configured(
        &mut self,
        document: &Value,
        name: Option<&str>,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        origin: Pos,
        _environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        self.calls
            .push((name.map(str::to_owned), origin, random.next_long()));
        world.set_feature_block(
            origin,
            document["config"]["state"].as_u64().unwrap() as u32,
            2,
        );
        if document["config"]["error"] == true {
            return Err(FeatureError::Unsupported("terminal callback".into()));
        }
        Ok(document["config"]["result"].as_bool().unwrap())
    }
}

#[test]
fn sequence_keeps_nested_order_origin_rng_and_partial_writes_at_first_failure() {
    let origin = (-3, 65, 19);
    for error in [false, true] {
        let document = json!({
            "type": "minecraft:sequence",
            "config": {"features": [
                {"feature": {"type": "probe:terminal", "config": {"state": 1, "result": true}}, "placement": []},
                {"feature": {"type": "probe:terminal", "config": {"state": 2, "result": false, "error": error}}, "placement": []},
                {"feature": {"type": "probe:terminal", "config": {"state": 3, "result": true}}, "placement": []}
            ]}
        });
        let mut world = World::default();
        let mut random = WorldgenRandom::new(42);
        let mut expected = WorldgenRandom::new(42);
        let mut dispatcher = BaseFeatures::with_dispatcher(Terminal::default());
        let result = dispatcher.place_configured(
            &document,
            Some("sequence_probe"),
            &mut world,
            &mut random,
            origin,
            &RegistryEnvironment,
        );
        assert_eq!(
            result,
            if error {
                Err(FeatureError::Unsupported("terminal callback".into()))
            } else {
                Ok(false)
            }
        );
        assert_eq!(
            dispatcher.additional.calls,
            [
                (None, origin, expected.next_long()),
                (None, origin, expected.next_long()),
            ]
        );
        assert_eq!(world.writes, [(origin, 1, 2), (origin, 2, 2)]);
        assert_eq!(random.next_long(), expected.next_long());
    }
}

#[test]
fn empty_sequence_is_rejected_by_the_native_composite_codec_without_a_draw() {
    let mut world = World::default();
    let mut random = WorldgenRandom::new(17);
    let mut expected = WorldgenRandom::new(17);
    assert!(matches!(
        BaseFeatures::default().place_configured(
            &json!({"type": "minecraft:sequence", "config": {"features": []}}),
            None,
            &mut world,
            &mut random,
            (8, 65, 8),
            &RegistryEnvironment,
        ),
        Err(FeatureError::InvalidConfig(_))
    ));
    assert!(world.writes.is_empty());
    assert_eq!(random.next_long(), expected.next_long());
}

#[test]
fn weighted_selector_retains_singleton_draw_and_flat_compact_entry_order() {
    for weights in [[1, 0], [3, 2], [63, 1], [0, 64]] {
        let total = weights.iter().sum::<usize>();
        for seed in [0, 1, 17, 42] {
            let origin = (-3, 65, 19);
            let document = json!({
                "type": "minecraft:weighted_random_selector",
                "config": {"features": [
                    {"weight": weights[0], "data": {"feature": {"type": "probe:terminal", "config": {"state": 1, "result": true}}, "placement": []}},
                    {"weight": weights[1], "data": {"feature": {"type": "probe:terminal", "config": {"state": 2, "result": false}}, "placement": []}}
                ]}
            });
            let mut world = World::default();
            let mut random = WorldgenRandom::new(seed);
            let mut expected = WorldgenRandom::new(seed);
            let selected = usize::from(expected.next_int(total) >= weights[0]);
            let mut dispatcher = BaseFeatures::with_dispatcher(Terminal::default());
            assert_eq!(
                dispatcher
                    .place_configured(
                        &document,
                        None,
                        &mut world,
                        &mut random,
                        origin,
                        &RegistryEnvironment
                    )
                    .unwrap(),
                selected == 0
            );
            assert_eq!(
                dispatcher.additional.calls,
                [(None, origin, expected.next_long())]
            );
            assert_eq!(world.writes, [(origin, selected as u32 + 1, 2)]);
            assert_eq!(random.next_long(), expected.next_long());
        }
    }
}

#[test]
fn zero_weight_and_invalid_weight_selectors_do_not_draw_or_write() {
    for features in [
        json!([]),
        json!([{"weight": 0, "data": {"feature": {"type": "probe:terminal", "config": {"state": 1, "result": true}}, "placement": []}}]),
    ] {
        let mut world = World::default();
        let mut random = WorldgenRandom::new(17);
        let mut expected = WorldgenRandom::new(17);
        assert_eq!(BaseFeatures::default().place_configured(
            &json!({"type": "minecraft:weighted_random_selector", "config": {"features": features}}),
            None, &mut world, &mut random, (8, 65, 8), &RegistryEnvironment), Ok(false));
        assert!(world.writes.is_empty());
        assert_eq!(random.next_long(), expected.next_long());
    }
    assert!(super::weighted_features(
        &json!({"features": [{"weight": -1, "data": "minecraft:oak_checked"}]})
    )
    .is_err());
    assert!(super::weighted_features(&json!({"features": [
        {"weight": i32::MAX, "data": "minecraft:oak_checked"},
        {"weight": 1, "data": "minecraft:oak_checked"}
    ]}))
    .is_err());
}

#[test]
fn selector_codecs_reject_malformed_children_before_admission_or_randomness() {
    for document in [
        json!({"type": "minecraft:weighted_random_selector", "config": {"features": [
            {"weight": 0, "data": null}
        ]}}),
        json!({"type": "minecraft:sequence", "config": {"features": [null]}}),
    ] {
        let mut world = World {
            denied: true,
            ..Default::default()
        };
        let mut random = WorldgenRandom::new(17);
        let mut expected = WorldgenRandom::new(17);
        assert!(matches!(
            BaseFeatures::default().place_configured(
                &document,
                None,
                &mut world,
                &mut random,
                (8, 65, 8),
                &RegistryEnvironment
            ),
            Err(FeatureError::InvalidConfig(_))
        ));
        assert!(world.writes.is_empty());
        assert_eq!(random.next_long(), expected.next_long());
    }
}

#[test]
fn lake_preflight_parses_all_target_predicates_and_explicit_legacy_fields() {
    let providers = json!({
        "fluid": {"type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:lava"}},
        "barrier": {"type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:stone"}}
    });
    check_lake_config(&providers).unwrap();
    let mut config = json!({
        "fluid": providers["fluid"], "barrier": providers["barrier"],
        "can_place_feature": {"type": "minecraft:true"},
        "can_replace_with_air_or_fluid": {
            "type": "minecraft:not",
            "predicate": {"type": "minecraft:matching_block_tag", "tag": "minecraft:features_cannot_replace"}
        },
        "can_replace_with_barrier": {
            "type": "minecraft:not",
            "predicate": {"type": "minecraft:matching_block_tag", "tag": "minecraft:lava_pool_stone_cannot_replace"}
        }
    });
    check_lake_config(&config).unwrap();
    config["can_replace_with_barrier"]["predicate"]["offset"] = json!([0, 0, 0]);
    check_lake_config(&config).unwrap();
    config["can_replace_with_barrier"]["predicate"]["offset"] = json!([0, 1, 0]);
    config["can_place_feature"] = json!({
        "type": "minecraft:not",
        "predicate": {"type": "minecraft:matching_blocks", "blocks": "minecraft:sulfur_spike"}
    });
    check_lake_config(&config).unwrap();
    config
        .as_object_mut()
        .unwrap()
        .remove("can_replace_with_air_or_fluid");
    assert!(matches!(
        check_lake_config(&config),
        Err(FeatureError::InvalidConfig(_))
    ));
}

#[test]
fn unsupported_lake_predicates_are_rejected_before_world_admission_or_draws() {
    for denied in [false, true] {
        let mut world = World {
            denied,
            ..Default::default()
        };
        let mut random = WorldgenRandom::new(17);
        let mut expected = WorldgenRandom::new(17);
        assert!(matches!(
            BaseFeatures::default().place_configured(
                &json!({"type": "minecraft:lake", "config": {
                    "can_place_feature": {"type": "probe:predicate"},
                    "can_replace_with_air_or_fluid": {"type": "minecraft:true"},
                    "can_replace_with_barrier": {"type": "minecraft:true"}
                }}),
                None,
                &mut world,
                &mut random,
                (8, 65, 8),
                &RegistryEnvironment,
            ),
            Err(FeatureError::Unsupported(_))
        ));
        assert!(world.writes.is_empty());
        assert_eq!(random.next_long(), expected.next_long());
    }
}

#[test]
fn biome_predicate_uses_the_native_position_and_keeps_short_circuiting() {
    use crate::block_predicate::BlockPredicate;
    let pos = (-3, 77, 19);
    let mut world = World::default();
    let predicate = BlockPredicate::parse(&json!({
        "type": "minecraft:matching_biomes", "biomes": "minecraft:plains",
        "offset": "ignored by the native matching-biomes codec"
    }))
    .unwrap();
    assert_eq!(predicate.test(&world, pos, &RegistryEnvironment), Ok(true));
    assert_eq!(*world.biome_reads.borrow(), [pos]);
    world.biome_reads.borrow_mut().clear();
    world.biome = Some(bcore_registry::BIOME_COUNT as u32);
    assert_eq!(
        BlockPredicate::Any(vec![BlockPredicate::True, predicate.clone()]).test(
            &world,
            pos,
            &RegistryEnvironment
        ),
        Ok(true)
    );
    assert!(world.biome_reads.borrow().is_empty());
    assert!(matches!(
        predicate.test(&world, pos, &RegistryEnvironment),
        Err(FeatureError::MissingData(_))
    ));
    assert!(matches!(
        BlockPredicate::parse(&json!({
            "type": "minecraft:matching_biomes", "biomes": "#probe:missing_tag"
        })),
        Err(FeatureError::MissingData(_))
    ));
}

#[test]
fn sequence_with_a_single_resource_holder_consumes_no_selector_draw() {
    let mut world = World::default();
    let mut random = WorldgenRandom::new(17);
    let mut expected = WorldgenRandom::new(17);
    let origin = (8, 65, 8);
    assert_eq!(
        BaseFeatures::default().place_configured(
            &json!({"type": "minecraft:sequence", "config": {"features": "minecraft:oak_checked"}}),
            None,
            &mut world,
            &mut random,
            origin,
            &RegistryEnvironment
        ),
        Ok(false)
    );
    assert!(world.writes.is_empty());
    assert_eq!(random.next_long(), expected.next_long());
}

#[test]
fn lake_admission_rejection_retains_shape_draws_and_precedes_fill_and_barrier() {
    let config = json!({
        "type": "minecraft:lake", "config": {
            "fluid": {"type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:air"}},
            "barrier": {"type": "minecraft:weighted_state_provider", "entries": [
                {"weight": 1, "data": {"Name": "minecraft:air"}}
            ]},
            "can_place_feature": {"type": "minecraft:matching_biomes", "biomes": "minecraft:plains"},
            "can_replace_with_air_or_fluid": {"type": "minecraft:true"},
            "can_replace_with_barrier": {"type": "minecraft:true"}
        }
    });
    for seed in [0, 17, 42] {
        let mut world = World {
            biome: Some(bcore_registry::biome::TAIGA),
            ..Default::default()
        };
        let mut random = WorldgenRandom::new(seed);
        let mut expected = WorldgenRandom::new(seed);
        let ellipsoids = expected.next_int(4) + 4;
        for _ in 0..ellipsoids {
            for _ in 0..6 {
                expected.next_double();
            }
        }
        assert_eq!(
            BaseFeatures::default().place_configured(
                &config,
                None,
                &mut world,
                &mut random,
                (8, 65, 8),
                &RegistryEnvironment
            ),
            Ok(false)
        );
        assert_eq!(world.biome_reads.borrow().len(), 1);
        assert!(world.writes.is_empty());
        assert_eq!(random.next_long(), expected.next_long());
    }
}
