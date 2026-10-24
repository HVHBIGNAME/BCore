use super::{minimum_size, parse_tree, tree, Predicate, Provider};
use crate::feature_world::FeatureError;
use crate::tree::fallen::{FallenTreeWorld, Pos};
use crate::tree::standing::StandingTreeWorld;
use crate::tree::{FeatureSize, TreeRandom};
use serde_json::json;
use std::cell::RefCell;

#[derive(Default)]
struct World {
    biome: Option<u32>,
    reads: RefCell<Vec<Pos>>,
    biome_reads: RefCell<Vec<Pos>>,
}

impl FallenTreeWorld for World {
    fn get_block(&self, pos: Pos) -> u32 {
        self.reads.borrow_mut().push(pos);
        crate::block::STONE
    }
    fn set_block(&mut self, _pos: Pos, _state: u32, _flags: i32) -> bool {
        panic!("provider evaluation must not write")
    }
    fn is_face_sturdy_up(&self, _state: u32, _pos: Pos) -> bool {
        panic!("unexpected support query")
    }
    fn mark_for_postprocessing(&mut self, _pos: Pos) {
        panic!("unexpected postprocessing")
    }
}

impl StandingTreeWorld for World {
    fn standing_biome(&self, pos: Pos) -> Result<u32, FeatureError> {
        self.biome_reads.borrow_mut().push(pos);
        self.biome
            .ok_or_else(|| FeatureError::MissingData("test biome lookup".into()))
    }
    fn has_beehive(&mut self, _pos: Pos) -> bool {
        panic!("unexpected hive lookup")
    }
    fn store_bee(&mut self, _pos: Pos, _ticks: i32) {
        panic!("unexpected hive mutation")
    }
    fn schedule_tree_tick(&mut self, _request: [i32; 6]) {
        panic!("unexpected tick request")
    }
}

#[derive(Default)]
struct Random(Vec<i32>);
impl TreeRandom for Random {
    fn next_i32_bounded(&mut self, bound: i32) -> i32 {
        self.0.push(bound);
        0
    }
    fn next_f32(&mut self) -> f32 {
        panic!("unexpected floating-point draw")
    }
}

#[test]
fn omitted_target_feature_size_fields_use_the_native_codec_defaults() {
    assert_eq!(
        minimum_size(&json!({"type": "minecraft:two_layers_feature_size"})).unwrap(),
        FeatureSize::TwoLayers {
            limit: 1,
            lower_size: 0,
            upper_size: 1
        }
    );
    assert_eq!(
        minimum_size(&json!({"type": "minecraft:three_layers_feature_size"})).unwrap(),
        FeatureSize::ThreeLayers {
            limit: 1,
            upper_limit: 1,
            lower_size: 0,
            middle_size: 1,
            upper_size: 1
        }
    );
    assert_eq!(
        minimum_size(&json!({"type": "minecraft:two_layers_feature_size", "limit": 81.0})).unwrap(),
        FeatureSize::TwoLayers {
            limit: 81,
            lower_size: 0,
            upper_size: 1
        }
    );
    for value in [
        json!({"type": "minecraft:two_layers_feature_size", "limit": 82}),
        json!({"type": "minecraft:two_layers_feature_size", "lower_size": -1}),
        json!({"type": "minecraft:three_layers_feature_size", "upper_limit": 81}),
        json!({"type": "minecraft:three_layers_feature_size", "middle_size": 17}),
        json!({"type": "minecraft:two_layers_feature_size", "upper_size": 1.5}),
    ] {
        assert!(matches!(
            minimum_size(&value),
            Err(FeatureError::InvalidConfig(_))
        ));
    }
}

#[test]
fn matching_biomes_queries_the_supplied_position_and_logical_predicates_are_lazy() {
    let pos = (-3, 77, 19);
    let predicate = Predicate::parse(&json!({
        "type": "minecraft:matching_biomes", "biomes": ["minecraft:plains"],
        "offset": "not part of this codec"
    }))
    .unwrap();
    let mut world = World {
        biome: Some(bcore_registry::biome::PLAINS),
        ..Default::default()
    };
    assert_eq!(predicate.test(&world, pos), Ok(true));
    assert_eq!(*world.biome_reads.borrow(), [pos]);
    assert!(world.reads.borrow().is_empty());
    world.biome_reads.borrow_mut().clear();
    world.biome = None;
    assert_eq!(
        Predicate::Any(vec![Predicate::True, predicate]).test(&world, pos),
        Ok(true)
    );
    assert!(world.biome_reads.borrow().is_empty());

    let predicate = Predicate::All(vec![
        Predicate::Not(Box::new(Predicate::True)),
        Predicate::Biomes([bcore_registry::biome::PLAINS].into()),
    ]);
    assert_eq!(predicate.test(&world, pos), Ok(false));
    assert!(world.biome_reads.borrow().is_empty());
    world.biome = Some(bcore_registry::BIOME_COUNT as u32);
    assert!(matches!(
        Predicate::Biomes(Default::default()).test(&world, pos),
        Err(FeatureError::MissingData(_))
    ));
}

#[test]
fn rule_provider_stops_at_the_first_match_and_weight_one_still_draws() {
    let provider = Provider::parse(&json!({
        "type": "minecraft:rule_based_state_provider",
        "fallback": {"type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:dirt"}},
        "rules": [
            {"if_true": {"type": "minecraft:matching_biomes", "biomes": "minecraft:plains"},
             "then": {"type": "minecraft:weighted_state_provider", "entries": [
                 {"data": {"Name": "minecraft:stone"}, "weight": 1.0}
             ]}},
            {"if_true": {"type": "minecraft:matching_blocks", "blocks": "stone"},
             "then": {"type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:dirt"}}}
        ]
    })).unwrap();
    let pos = (8, 64, 8);
    let world = World {
        biome: Some(bcore_registry::biome::PLAINS),
        ..Default::default()
    };
    let mut random = Random::default();
    assert_eq!(
        provider.sample(&world, &mut random, pos),
        Ok(Some(crate::block::STONE))
    );
    assert_eq!(*world.biome_reads.borrow(), [pos]);
    assert!(world.reads.borrow().is_empty());
    assert_eq!(random.0, [1]);
}

#[test]
fn randomized_provider_keeps_unknown_or_noninteger_properties_without_sampling_values() {
    for (block, property, expected) in [
        ("stone", "age", crate::block::STONE),
        ("oak_leaves", "persistent", crate::block::OAK_LEAVES),
    ] {
        let provider = Provider::parse(&json!({
            "type": "minecraft:randomized_int_state_provider",
            "source": {"type": "minecraft:simple_state_provider", "state": {"Name": format!("minecraft:{block}")}},
            "property": property, "values": {"type": "minecraft:uniform", "min_inclusive": 0, "max_inclusive": 1}
        })).unwrap();
        let world = World::default();
        let mut random = Random::default();
        assert_eq!(
            provider.sample(&world, &mut random, (8, 65, 8)),
            Ok(Some(expected))
        );
        assert!(random.0.is_empty());
        assert!(world.reads.borrow().is_empty());
    }
}

#[test]
fn rule_provider_distinguishes_optional_absence_from_required_and_nested_reads() {
    let world = World::default();
    let pos = (-3, 77, 19);
    let mut random = Random::default();
    let absent = json!({"type": "minecraft:rule_based_state_provider", "rules": []});
    let provider = Provider::parse(&absent).unwrap();
    assert_eq!(provider.sample(&world, &mut random, pos), Ok(None));
    assert!(world.reads.borrow().is_empty());
    assert_eq!(
        provider.sample_required(&world, &mut random, pos),
        Ok(crate::block::STONE)
    );
    assert_eq!(*world.reads.borrow(), [pos]);
    world.reads.borrow_mut().clear();
    for value in [
        json!({"type": "minecraft:rule_based_state_provider", "fallback": absent, "rules": []}),
        json!({"type": "minecraft:rule_based_state_provider", "rules": [
            {"if_true": {"type": "minecraft:true"}, "then": absent}
        ]}),
    ] {
        assert_eq!(
            Provider::parse(&value)
                .unwrap()
                .sample(&world, &mut random, pos),
            Ok(Some(crate::block::STONE))
        );
        assert_eq!(*world.reads.borrow(), [pos]);
        world.reads.borrow_mut().clear();
    }
    assert!(random.0.is_empty());
}

#[test]
fn target_clipped_height_and_ground_decorator_defaults_obey_native_codec_bounds() {
    use crate::tree::standing::extra_decorators::Decorator;

    let provider = json!({"type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:leaf_litter"}});
    assert!(matches!(
        Decorator::parse(&json!({
            "type": "minecraft:place_on_ground", "block_state_provider": provider,
        }))
        .unwrap(),
        Decorator::PlaceOnGround {
            radius: 2,
            height: 1,
            tries: 128,
            ..
        }
    ));
    for (name, value) in [("radius", -1), ("height", -1), ("tries", 0)] {
        let mut document =
            json!({"type": "minecraft:place_on_ground", "block_state_provider": provider});
        document[name] = json!(value);
        assert!(matches!(
            Decorator::parse(&document),
            Err(FeatureError::InvalidConfig(_))
        ));
    }
    let mut config = crate::block_predicate::catalog().configured("oak").unwrap()["config"].clone();
    for height in [0, 80] {
        config["minimum_size"]["min_clipped_height"] = json!(height);
        assert_eq!(
            parse_tree(&config).unwrap().shape.min_clipped_height,
            Some(height)
        );
    }
    for height in [-1, 81] {
        config["minimum_size"]["min_clipped_height"] = json!(height);
        assert!(matches!(
            parse_tree(&config),
            Err(FeatureError::InvalidConfig(_))
        ));
    }
    for name in [
        "oak_leaf_litter",
        "birch_leaf_litter",
        "fancy_oak_leaf_litter",
        "dark_oak_leaf_litter",
    ] {
        assert!(tree(name).unwrap().is_some(), "target tree {name}");
    }
}

#[test]
fn requested_tree_schema_errors_do_not_poison_an_unrelated_catalog_tree() {
    let mut config = crate::block_predicate::catalog().configured("oak").unwrap()["config"].clone();
    config["decorators"] = json!([{"type": "probe:unsupported_decorator"}]);
    assert!(matches!(
        parse_tree(&config),
        Err(FeatureError::Unsupported(_))
    ));
    config["decorators"] = json!([]);
    config["trunk_placer"]["can_grow_through"] = json!("#probe:missing_tag");
    assert!(matches!(
        parse_tree(&config),
        Err(FeatureError::MissingData(_))
    ));
    assert!(tree("minecraft:birch").unwrap().is_some());
    assert!(tree("probe:birch").unwrap().is_none());
    config["trunk_placer"]
        .as_object_mut()
        .unwrap()
        .remove("can_grow_through");
    config["ignore_vines"] = json!("invalid native BOOL, defaults through orElse");
    assert!(!parse_tree(&config).unwrap().ignore_vines);
}

#[test]
fn soil_provider_failure_stops_before_trunk_and_foliage_draws() {
    struct Sink {
        failed: bool,
        soil_calls: usize,
    }
    impl crate::tree::ShapeSink for Sink {
        fn failed(&self) -> bool {
            self.failed
        }
        fn state(&self, _pos: Pos) -> Option<u32> {
            Some(crate::block::AIR)
        }
        fn valid_state(&self, _state: u32) -> bool {
            true
        }
        fn free_state(&self, _state: u32) -> bool {
            true
        }
        fn log(&mut self, _pos: Pos, _state: u32) -> bool {
            panic!("soil failure must prevent trunk writes")
        }
        fn leaf(&mut self, _pos: Pos, _state: u32) -> bool {
            panic!("soil failure must prevent foliage writes")
        }
        fn below_trunk<R: TreeRandom + ?Sized>(&mut self, _random: &mut R, _pos: Pos) {
            self.soil_calls += 1;
            self.failed = true;
        }
        fn leaf_is_set(&self, _pos: Pos) -> bool {
            false
        }
    }
    for (config, draws) in [
        (crate::tree::OAK, vec![3, 1]),
        (crate::tree::ACACIA, vec![3, 3]),
        (crate::tree::FANCY_OAK, vec![12, 1]),
        (crate::tree::DARK_OAK, vec![3, 2]),
    ] {
        let mut sink = Sink {
            failed: false,
            soil_calls: 0,
        };
        let mut random = Random::default();
        assert!(!crate::tree::grow_shape(
            &mut sink,
            &mut random,
            &config,
            (8, 65, 8)
        ));
        assert_eq!(sink.soil_calls, 1);
        assert_eq!(random.0, draws);
    }
}
