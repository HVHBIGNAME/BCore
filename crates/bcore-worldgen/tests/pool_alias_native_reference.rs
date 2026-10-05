//! Immutable output from real 26.1 binding codecs, callbacks and alias lookups.
use bcore_worldgen::simplex::JavaRandom;
use bcore_worldgen::structure::pool_alias::{positional_random, PoolAliasBindings};
use bcore_worldgen::structure::template::TemplateRandom;
use bcore_worldgen::structure::template_pool::StructureAssets;
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../data/pool_alias_bindings_26_1_v2.json")).unwrap()
}

fn position(value: &Value) -> (i32, i32, i32) {
    let [x, y, z]: [i32; 3] = serde_json::from_value(value.clone()).unwrap();
    (x, y, z)
}

struct Traced {
    source: JavaRandom,
    draws: Vec<[usize; 2]>,
}

impl Traced {
    fn new(sample: &Value) -> Self {
        Self {
            source: positional_random(sample["seed"].as_i64().unwrap(), position(&sample["pos"])),
            draws: Vec::new(),
        }
    }
}

impl TemplateRandom for Traced {
    fn next_int(&mut self, bound: usize) -> usize {
        let value = self.source.next_int(bound);
        self.draws.push([bound, value]);
        value
    }

    fn next_long(&mut self) -> i64 {
        panic!("native aliases only call nextInt(bound)")
    }

    fn next_float(&mut self) -> f32 {
        panic!("native aliases only call nextInt(bound)")
    }
}

#[test]
fn native_codec_acceptance_encoding_and_all_targets() {
    let fixture = fixture();
    assert_eq!(fixture["jar_sha256"], StructureAssets::JAR_SHA256);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 31);
    let mut accepted = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let parsed = PoolAliasBindings::from_json(&case["input"]);
        if case.get("codec_error").is_some() {
            assert!(
                parsed.is_err(),
                "native rejects {name}: {}",
                case["codec_error"]
            );
            assert!(serde_json::from_value::<PoolAliasBindings>(case["input"].clone()).is_err());
            continue;
        }
        let bindings = parsed.unwrap_or_else(|error| panic!("{name}: {error:?}"));
        accepted += 1;
        assert_eq!(
            bindings.to_json(),
            case["encoded"],
            "native encoding: {name}"
        );
        assert_eq!(
            serde_json::to_value(&bindings).unwrap(),
            case["encoded"],
            "serde: {name}"
        );
        assert_eq!(
            serde_json::from_value::<PoolAliasBindings>(case["encoded"].clone()).unwrap(),
            bindings,
            "round trip: {name}"
        );
        assert_eq!(
            serde_json::to_value(bindings.all_targets()).unwrap(),
            case["all_targets"],
            "targets: {name}"
        );
    }
    assert_eq!(accepted, 16);
}

#[test]
fn optional_pool_alias_field_matches_native_jigsaw_codec() {
    let fixture = fixture();
    let cases = fixture["structure_alias_fields"].as_array().unwrap();
    assert_eq!(cases.len(), 4);
    for case in cases {
        let parsed = PoolAliasBindings::from_structure_json(&case["input"]);
        if case.get("codec_error").is_some() {
            assert!(
                parsed.is_err(),
                "{}: native rejects malformed optional aliases",
                case["name"]
            );
        } else {
            let bindings = parsed.unwrap();
            assert!(bindings.is_empty());
            assert_eq!(bindings.len(), 0);
            assert_eq!(bindings.to_json(), case["encoded"], "{}", case["name"]);
        }
    }
}

#[test]
fn native_positional_callback_order_draws_and_rng_tails() {
    let fixture = fixture();
    let mut checked = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let Some(samples) = case["samples"].as_array() else {
            continue;
        };
        let bindings = PoolAliasBindings::from_json(&case["input"]).unwrap();
        for sample in samples {
            let description = format!(
                "{} seed={} pos={}",
                case["name"], sample["seed"], sample["pos"]
            );
            let mut random = Traced::new(sample);
            let mut pairs = Vec::new();
            bindings.for_each_resolved(&mut random, |alias, target| {
                pairs.push([alias.to_owned(), target.to_owned()]);
            });
            assert_eq!(
                serde_json::to_value(pairs).unwrap(),
                sample["ordered"],
                "{description} pairs"
            );
            assert_eq!(
                serde_json::to_value(&random.draws).unwrap(),
                sample["draws"],
                "{description} draws"
            );
            assert_eq!(
                random.source.next_long(),
                sample["next_i64"].as_i64().unwrap(),
                "{description} child tail"
            );
            let mut parent = JavaRandom::new(sample["seed"].as_i64().unwrap());
            parent.next_long();
            assert_eq!(
                parent.next_long(),
                sample["parent_next_i64"].as_i64().unwrap(),
                "{description} fork tail"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 672);
}

#[test]
fn native_lookup_is_single_pass_and_duplicate_failure_consumes_all_bindings() {
    let fixture = fixture();
    let mut rejected = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let Some(samples) = case["samples"].as_array() else {
            continue;
        };
        let bindings = PoolAliasBindings::from_json(&case["input"]).unwrap();
        for sample in samples {
            let description = format!(
                "{} seed={} pos={}",
                case["name"], sample["seed"], sample["pos"]
            );
            let mut random = Traced::new(sample);
            let resolved = bindings.resolve_with_random(&mut random);
            let seeded =
                bindings.resolve(sample["seed"].as_i64().unwrap(), position(&sample["pos"]));
            assert_eq!(
                serde_json::to_value(&random.draws).unwrap(),
                sample["draws"],
                "{description} draws before build"
            );
            assert_eq!(
                random.source.next_long(),
                sample["next_i64"].as_i64().unwrap(),
                "{description} tail after build"
            );
            if sample.get("lookup_error").is_some() {
                rejected += 1;
                assert!(
                    resolved.is_err(),
                    "{description}: native rejects duplicate aliases"
                );
                assert!(
                    seeded.is_err(),
                    "{description}: seeded lookup must also reject"
                );
                continue;
            }
            let resolved = resolved.unwrap();
            assert_eq!(seeded.unwrap(), resolved, "{description} source ownership");
            for (query, target) in sample["lookup"].as_object().unwrap() {
                assert_eq!(
                    resolved.get(query).unwrap_or(query),
                    target.as_str().unwrap(),
                    "{description} lookup {query}"
                );
            }
        }
    }
    // Two unconditional duplicate configurations plus both selected branches of
    // the native conditional-duplicate group must be exercised.
    assert!(rejected > 84 && rejected < 126);
}
