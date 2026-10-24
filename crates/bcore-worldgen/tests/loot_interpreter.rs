//! Structural/boundary tests for catalogs not present in the native case matrix.
//! These supplement the native oracle; they do not add native execution coverage.
use bcore_worldgen::loot::{
    CatalogSources, ItemStack, LegacyRandom, LootContext, LootEngine, LootError, LootRng,
};
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::Arc};

#[derive(Debug, PartialEq)]
enum Draw {
    Int(i32, i32),
    Float(f32),
    Bool(bool),
}
struct Scripted(VecDeque<Draw>);
impl Scripted {
    fn new(draws: impl IntoIterator<Item = Draw>) -> Self {
        Self(draws.into_iter().collect())
    }
    fn finished(&self) {
        assert!(self.0.is_empty(), "unused draws: {:?}", self.0);
    }
}
impl LootRng for Scripted {
    fn next_int(&mut self, bound: i32) -> i32 {
        let Some(Draw::Int(expected, value)) = self.0.pop_front() else {
            panic!("unexpected nextInt({bound})");
        };
        assert_eq!(bound, expected);
        assert!((0..bound).contains(&value));
        value
    }
    fn next_float(&mut self) -> f32 {
        let Some(Draw::Float(value)) = self.0.pop_front() else {
            panic!("unexpected nextFloat");
        };
        value
    }
    fn next_bool(&mut self) -> bool {
        let Some(Draw::Bool(value)) = self.0.pop_front() else {
            panic!("unexpected nextBoolean");
        };
        value
    }
}

fn catalog(tables: Value) -> LootEngine {
    LootEngine::from_sources(CatalogSources {
        tables: &tables.to_string(),
        ..CatalogSources::VANILLA
    })
    .unwrap()
}
fn item(name: &str, functions: Value) -> Value {
    json!({"type":"minecraft:item", "name":name, "functions":functions})
}
fn single(entry: Value) -> Value {
    json!({"pools":[{"rolls":1,"entries":[entry]}]})
}
fn uniform(min: i32, max: i32) -> Value {
    json!({"type":"minecraft:uniform", "min":min, "max":max})
}
fn count(number: Value, add: bool) -> Value {
    json!({"function":"minecraft:set_count", "count":number, "add":add})
}
fn generate(engine: &LootEngine, key: &str, rng: &mut dyn LootRng) -> Vec<ItemStack> {
    engine
        .table(key)
        .unwrap()
        .generate_with_rng(LootContext::CHEST, rng)
        .unwrap()
}

#[test]
fn nested_emissions_apply_outer_functions_before_the_next_child_roll() {
    let child = json!({
        "random_sequence":"test:child_sequence",
        "functions":[count(uniform(1,5), true)],
        "pools":[{"rolls":2,"functions":[count(json!(1), true)],
            "entries":[item("stone", json!([count(uniform(1,3), false)]))]}]
    });
    let root = json!({
        "random_sequence":"test:root_sequence",
        "functions":[count(json!(1),true)],
        "pools":[{"rolls":1,"bonus_rolls":uniform(0,1),"functions":[count(json!(1),true)],
            "entries":[{"type":"minecraft:loot_table","value":"test:child",
                "functions":[count(uniform(1,7),true)]}]}]
    });
    let engine = catalog(json!({"test:root":root,"test:child":child}));
    let table = engine.table("test:root").unwrap();
    assert_eq!(table.random_sequence(), Some("test:root_sequence"));
    let mut rng = Scripted::new([
        Draw::Float(0.625),
        Draw::Int(3, 2),
        Draw::Int(5, 0),
        Draw::Int(7, 1),
        Draw::Int(3, 0),
        Draw::Int(5, 2),
        Draw::Int(7, 0),
    ]);
    let result = table
        .generate_with_rng(LootContext::CHEST, &mut rng)
        .unwrap();
    rng.finished();
    assert_eq!(
        result.iter().map(ItemStack::count).collect::<Vec<_>>(),
        [9, 8]
    );
    assert!(Arc::ptr_eq(result[0].definition(), result[1].definition()));
}

#[test]
fn conditions_evaluate_providers_first_and_short_circuit() {
    let engine = catalog(json!({"test:conditions":{"pools":[
    {"conditions":[{"condition":"minecraft:random_chance","chance":uniform(0,1)}],
        "rolls":uniform(100,200),"entries":[item("diamond",json!([]))]},
    {"rolls":1,"bonus_rolls":uniform(0,1),"entries":[
        {"type":"minecraft:alternatives","children":[
            {"type":"minecraft:item","name":"stone","conditions":[
                {"condition":"minecraft:all_of","terms":[
                    {"condition":"minecraft:random_chance","chance":0},
                    {"condition":"minecraft:random_chance","chance":1}]}]},
            {"type":"minecraft:item","name":"emerald","conditions":[
                {"condition":"minecraft:inverted","term":{"condition":"minecraft:random_chance","chance":0}}]},
            {"type":"minecraft:item","name":"diamond","conditions":[{"condition":"minecraft:random_chance","chance":1}]}
        ]}]}]}}));
    let mut rng = Scripted::new([
        Draw::Float(0.1),
        Draw::Float(0.2),
        Draw::Float(0.5),
        Draw::Float(0.5),
        Draw::Float(0.5),
    ]);
    let result = generate(&engine, "test:conditions", &mut rng);
    rng.finished();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id(), "minecraft:emerald");
}

#[test]
fn empty_weighted_entries_compete_but_a_singleton_skips_selection() {
    let engine = catalog(json!({"test:entries":{"pools":[
        {"rolls":1,"entries":[{"type":"minecraft:empty","weight":2},
            {"type":"minecraft:item","name":"diamond","weight":1}]},
        {"rolls":1,"entries":[{"type":"minecraft:empty","weight":0},
            {"type":"minecraft:item","name":"emerald","weight":3}]}
    ]}}));
    let mut rng = Scripted::new([Draw::Int(3, 1)]);
    let result = generate(&engine, "test:entries", &mut rng);
    rng.finished();
    assert_eq!(
        result.iter().map(ItemStack::id).collect::<Vec<_>>(),
        ["minecraft:emerald"]
    );
}

#[test]
fn enchantment_weighted_choice_draws_even_for_one_candidate() {
    let engine = catalog(json!({"test:enchant":single(item("iron_sword",json!([
        {"function":"minecraft:enchant_with_levels","levels":30,"options":"minecraft:unbreaking"}
    ])))}));
    let mut rng = Scripted::new([
        Draw::Int(4, 0),
        Draw::Int(4, 0),
        Draw::Float(0.5),
        Draw::Float(0.5),
        Draw::Int(5, 0),
        Draw::Int(50, 49),
    ]);
    let result = generate(&engine, "test:enchant", &mut rng);
    rng.finished();
    assert_eq!(
        result[0].component("minecraft:enchantments"),
        Some(&json!({"minecraft:unbreaking":3}))
    );
}

#[test]
fn book_transmutation_distinguishes_fresh_and_copy_and_splits_only_after_functions() {
    let common = vec![
        count(json!(5), false),
        json!({"function":"minecraft:set_potion","id":"minecraft:healing"}),
    ];
    let mut fresh = common.clone();
    fresh.push(json!({"function":"minecraft:enchant_randomly","options":"minecraft:wind_burst"}));
    let mut copied = common;
    copied.push(json!({"function":"minecraft:set_enchantments","enchantments":{"minecraft:wind_burst":3},"add":false}));
    let engine = catalog(
        json!({"test:fresh":single(item("book",json!(fresh))),"test:copy":single(item("book",json!(copied)))}),
    );
    let mut rng = Scripted::new([Draw::Int(1, 0), Draw::Int(3, 2)]);
    let result = generate(&engine, "test:fresh", &mut rng);
    rng.finished();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id(), "minecraft:enchanted_book");
    assert!(result[0].component("minecraft:potion_contents").is_none());
    assert_eq!(result[0].count(), 1);
    let copied = generate(&engine, "test:copy", &mut rng);
    rng.finished();
    assert_eq!(copied.len(), 5);
    for stack in copied {
        assert_eq!(stack.count(), 1);
        assert_eq!(
            stack.component("minecraft:potion_contents"),
            Some(&json!({"potion":"minecraft:healing"}))
        );
        assert_eq!(
            stack.component("minecraft:stored_enchantments"),
            Some(&json!({"minecraft:wind_burst":3}))
        );
    }
}

#[test]
fn maximum_size_splitting_is_deterministic_and_default_patches_are_elided() {
    let engine = catalog(
        json!({"test:stack":single(item("stone",json!([count(json!(130),false)]))),
        "test:default":single(item("ominous_bottle",json!([{"function":"minecraft:set_ominous_bottle_amplifier","amplifier":-3}])))}),
    );
    let mut rng = Scripted::new([]);
    let result = generate(&engine, "test:stack", &mut rng);
    assert_eq!(
        result.iter().map(ItemStack::count).collect::<Vec<_>>(),
        [64, 64, 2]
    );
    let result = generate(&engine, "test:default", &mut rng);
    assert!(result[0].persistent_patch().is_empty());
    assert_eq!(
        result[0].component("minecraft:ominous_bottle_amplifier"),
        Some(&json!(0))
    );
    rng.finished();
}

#[test]
fn randomized_split_preserves_removal_order_and_skips_degenerate_draws() {
    let engine =
        catalog(json!({"test:split":single(item("stone",json!([count(json!(4),false)])))}));
    let mut rng = Scripted::new([
        Draw::Int(3, 0),
        Draw::Int(2, 0), // empty-slot shuffle
        Draw::Int(2, 1),
        Draw::Bool(true),
        Draw::Bool(false), // split 4 into two 2s
        Draw::Int(3, 1),
        Draw::Int(2, 1),
    ]); // final item shuffle; second split is all degenerate
    let mut slots = vec![None; 3];
    engine
        .table("test:split")
        .unwrap()
        .fill_with_rng(&mut slots, LootContext::CHEST, &mut rng)
        .unwrap();
    rng.finished();
    assert_eq!(slots.iter().flatten().map(ItemStack::count).sum::<i32>(), 4);
    assert!(slots.iter().all(Option::is_some));
}

#[test]
fn component_functions_keep_add_clamp_and_nondamageable_no_draw_semantics() {
    let engine = catalog(json!({
        "test:damage":single(item("iron_pickaxe",json!([
            {"function":"minecraft:set_damage","damage":0.25},
            {"function":"minecraft:set_damage","damage":0.25,"add":true}
        ]))),
        "test:nondamageable":single(item("stone",json!([
            {"function":"minecraft:set_damage","damage":uniform(0,1)}
        ]))),
        "test:enchantment_cap":single(item("book",json!([
            {"function":"minecraft:set_enchantments","enchantments":{"minecraft:wind_burst":250}},
            {"function":"minecraft:set_enchantments","enchantments":{"minecraft:wind_burst":10},"add":true}
        ]))),
        "test:enchantment_zero":single(item("book",json!([
            {"function":"minecraft:set_enchantments","enchantments":{"minecraft:wind_burst":3}},
            {"function":"minecraft:set_enchantments","enchantments":{"minecraft:wind_burst":-5},"add":true}
        ])))
    }));
    let mut rng = Scripted::new([]);
    let damaged = generate(&engine, "test:damage", &mut rng);
    assert_eq!(damaged[0].component("minecraft:damage"), Some(&json!(124)));
    let stone = generate(&engine, "test:nondamageable", &mut rng);
    assert!(stone[0].persistent_patch().is_empty());
    let enchanted = generate(&engine, "test:enchantment_cap", &mut rng);
    assert_eq!(
        enchanted[0].component("minecraft:stored_enchantments"),
        Some(&json!({"minecraft:wind_burst":255}))
    );
    let cleared = generate(&engine, "test:enchantment_zero", &mut rng);
    assert!(cleared[0].persistent_patch().is_empty());
    rng.finished();
}

#[test]
fn unsupported_contexts_fail_before_rng_or_slot_changes() {
    let engine = LootEngine::vanilla().unwrap();
    let table = engine.table("chests/simple_dungeon").unwrap();
    let stone = engine.item_stack("stone", 12).unwrap();
    let mut slots = vec![Some(stone.clone()), None];
    let before = slots.clone();
    assert_eq!(
        table.fill(&mut slots, 0),
        Err(LootError::ZeroSeedRequiresCallerRng)
    );
    let mut rng = Scripted::new([]);
    for context in [
        LootContext {
            luck: 1.0,
            player_present: false,
            ..LootContext::CHEST
        },
        LootContext {
            luck: -0.0,
            player_present: false,
            ..LootContext::CHEST
        },
        LootContext {
            luck: f32::NAN,
            player_present: false,
            ..LootContext::CHEST
        },
        LootContext {
            luck: 0.0,
            player_present: true,
            ..LootContext::CHEST
        },
    ] {
        assert!(matches!(
            table.fill_with_rng(&mut slots, context, &mut rng),
            Err(LootError::UnsupportedContext(_))
        ));
    }
    assert_eq!(slots, before);
    rng.finished();
    for key in [
        "chests/shipwreck_map",
        "chests/underwater_ruin_big",
        "chests/underwater_ruin_small",
    ] {
        assert!(matches!(
            engine.table(key),
            Err(LootError::Unsupported { .. })
        ));
    }
}

#[test]
fn zero_count_book_exposes_air_and_does_not_enchant_or_transmute() {
    let engine = catalog(json!({"test:zero":single(item("book",json!([
        count(json!(0),false),
        {"function":"minecraft:enchant_randomly","options":"minecraft:wind_burst"},
        {"function":"minecraft:enchant_with_levels","levels":uniform(3,5)},
        {"function":"minecraft:set_enchantments","enchantments":{"minecraft:wind_burst":uniform(1,3)}},
        count(json!(1),true)
    ])))}));
    // With-levels evaluates its level provider first; its helper then sees no
    // enchantable component. The other enchantment functions draw nothing.
    let mut rng = Scripted::new([Draw::Int(3, 0)]);
    let result = generate(&engine, "test:zero", &mut rng);
    rng.finished();
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].to_json(),
        json!({"id":"minecraft:book","count":1})
    );
    let empty = result[0].with_count(0);
    assert_eq!(empty.id(), "minecraft:air");
    assert_eq!(empty.numeric_id(), 0);
    assert_eq!(empty.effective_components().count(), 0);
    assert!(empty.component("minecraft:enchantable").is_none());
    assert!(empty.persistent_patch().is_empty());
    assert!(empty.with_count(1).is_empty());
}

#[test]
fn missing_and_inline_tables_share_caller_rng_without_reseeding() {
    let engine = catalog(
        json!({"test:inline":single(json!({"type":"minecraft:loot_table","value":
        {"random_sequence":"test:ignored","pools":[{"rolls":1,"entries":[item("stone",json!([count(uniform(2,5),false)]))]}]}}))}),
    );
    let mut rng = Scripted::new([Draw::Int(4, 2)]);
    let result = generate(&engine, "test:inline", &mut rng);
    assert_eq!(result[0].count(), 4);
    assert!(generate(&engine, "test:missing", &mut rng).is_empty());
    rng.finished();
    assert!(engine.table("test:missing").unwrap().is_missing());
    let mut source = LegacyRandom::new(0); // explicit source, not an optional seed
    generate(&engine, "test:inline", &mut source);
}

#[test]
fn unsupported_and_recursive_definitions_are_never_empty_fallbacks() {
    let engine = catalog(json!({
        "test:bad":single(item("stone",json!([{"function":"test:unknown"}]))),
        "test:parent":single(json!({"type":"minecraft:loot_table","value":"test:bad"})),
        "test:cycle_a":single(json!({"type":"minecraft:loot_table","value":"test:cycle_b"})),
        "test:cycle_b":single(json!({"type":"minecraft:loot_table","value":"test:cycle_a"})),
        "test:malformed":{"pools":[{"rolls":1,"entries":"wrong"}]},
        "test:wide":{"pools":[{"rolls":2147483647,"entries":[item("stone",json!([]))]}]}
    }));
    for key in ["test:bad", "test:parent"] {
        assert!(matches!(
            engine.table(key),
            Err(LootError::Unsupported { .. })
        ));
    }
    for key in ["test:cycle_a", "test:cycle_b"] {
        assert!(matches!(
            engine.table(key),
            Err(LootError::RecursiveTable(_))
        ));
    }
    assert!(matches!(
        engine.table("test:malformed"),
        Err(LootError::InvalidData { .. })
    ));
    assert!(matches!(
        engine.table("test:wide").unwrap().generate(1),
        Err(LootError::LimitExceeded(_))
    ));
    assert!(engine
        .item_stack_from_json(&json!({"id":"minecraft:stone","components":{"test:unknown":1}}))
        .is_err());
    assert!(engine
        .item_stack_from_json(
            &json!({"id":"minecraft:stone","components":{"!minecraft:enchantments":{}}})
        )
        .is_err());
}

#[test]
fn legacy_rng_matches_existing_java_random_across_rejection_and_signed_word_paths() {
    use bcore_worldgen::simplex::JavaRandom;
    for seed in [i64::MIN, -17, 0, 1, 52, i64::MAX] {
        let mut loot = LegacyRandom::new(seed);
        let mut existing = JavaRandom::new(seed);
        for _ in 0..32 {
            for bound in [1, 2, 3, 7, 32, 1000, 1_073_741_825, i32::MAX] {
                assert_eq!(
                    loot.next_int(bound),
                    LootRng::next_int(&mut existing, bound)
                );
            }
            assert_eq!(loot.next_bool(), LootRng::next_bool(&mut existing));
            assert_eq!(
                loot.next_float().to_bits(),
                LootRng::next_float(&mut existing).to_bits()
            );
            assert_eq!(loot.next_long(), existing.next_long());
            assert_eq!(
                loot.next_double().to_bits(),
                existing.next_double().to_bits()
            );
        }
    }
}
