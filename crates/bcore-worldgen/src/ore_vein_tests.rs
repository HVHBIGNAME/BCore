use super::*;
use crate::density_materials_tests::{bits, fixture, pos};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

#[test]
fn native_material_decisions_rng_continuations_and_lazy_reads() {
    let fixture = fixture(
        include_str!("../data/ore_vein_materials_26_1.json"),
        "materials",
    );
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 1598);
    let mut results = BTreeSet::new();
    let mut draw_counts = BTreeSet::new();
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let seed = case["seed"].as_i64().unwrap();
        let p = pos(&case["pos"]);
        let inputs: Vec<_> = case["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| f64::from_bits(bits(v)))
            .collect();
        let scripted: Option<Vec<_>> = case["scripted"].as_array().map(|values| {
            values
                .iter()
                .map(|v| f32::from_bits(bits(v) as u32))
                .collect()
        });
        let ore = OreVeinifier::new(seed);
        let rng = RefCell::new(ore.random.at(p.0, p.1, p.2));
        let events = RefCell::new(vec!["toggle"]);
        let count = Cell::new(0);
        let result = calculate_with_random(
            p.1,
            inputs[0],
            || {
                events.borrow_mut().push("ridged");
                inputs[1]
            },
            || {
                events.borrow_mut().push("gap");
                inputs[2]
            },
            || {
                events.borrow_mut().push("at");
                || {
                    events.borrow_mut().push("float");
                    let i = count.get();
                    count.set(i + 1);
                    scripted
                        .as_ref()
                        .map_or_else(|| rng.borrow_mut().next_float(), |values| values[i])
                }
            },
        );
        let expected = case["state"].as_i64().unwrap();
        assert_eq!(
            result.map_or(-1, i64::from),
            expected,
            "{id}: inputs {inputs:?} at {p:?}"
        );
        assert_eq!(
            serde_json::to_value(events.into_inner()).unwrap(),
            case["events"],
            "{id}: lazy evaluation/RNG order"
        );
        assert_eq!(
            count.get(),
            case["draws"].as_u64().unwrap() as usize,
            "{id}"
        );
        if let Some(next) = case["next_i64"].as_i64() {
            assert_eq!(
                rng.borrow_mut().next_long() as i64,
                next,
                "{id}: RNG continuation"
            );
        }
        if scripted.is_none() {
            assert_eq!(
                ore.calculate(p, inputs[0], || inputs[1], || inputs[2]),
                result,
                "{id}: positional factory path"
            );
        }
        results.insert(expected);
        draw_counts.insert(count.get());
    }
    assert_eq!(draw_counts, BTreeSet::from([0, 1, 2, 3]));
    assert_eq!(
        results,
        BTreeSet::from([
            -1,
            i64::from(block::GRANITE),
            i64::from(block::TUFF),
            i64::from(block::COPPER_ORE),
            i64::from(block::RAW_COPPER_BLOCK),
            i64::from(block::DEEPSLATE_IRON_ORE),
            i64::from(block::RAW_IRON_BLOCK)
        ])
    );
    for (name, state) in [
        ("COPPER_ORE", block::COPPER_ORE),
        ("RAW_COPPER_BLOCK", block::RAW_COPPER_BLOCK),
        ("GRANITE", block::GRANITE),
        ("DEEPSLATE_IRON_ORE", block::DEEPSLATE_IRON_ORE),
        ("RAW_IRON_BLOCK", block::RAW_IRON_BLOCK),
        ("TUFF", block::TUFF),
    ] {
        assert_eq!(fixture["states"][name].as_u64().unwrap(), state as u64);
    }
}
