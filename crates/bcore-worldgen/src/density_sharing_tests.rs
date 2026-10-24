use super::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

fn reference(f: &DensityFunction) -> &Arc<DensityFunction> {
    let DensityFunction::Reference(value) = f else {
        panic!("expected named reference: {f:?}");
    };
    value
}

#[test]
fn named_references_share_once_per_parse_but_separate_roots_and_inline_markers() {
    let input = r#"{"type":"minecraft:add","argument1":"a","argument2":"minecraft:a"}"#;
    let mut loads = Vec::new();
    let mut load = |path: &str| {
        loads.push(path.to_owned());
        Ok(r#"{"type":"minecraft:cache_once","argument":2.0}"#.to_owned())
    };
    let first = FunctionParser::new(&mut load).parse(input).unwrap();
    let second = FunctionParser::new(&mut load).parse(input).unwrap();
    assert_eq!(
        loads,
        ["density_function/a.json", "density_function/a.json"]
    );
    let DensityFunction::Add(a, b) = &first else {
        panic!()
    };
    let DensityFunction::Add(c, _) = &second else {
        panic!()
    };
    assert!(Arc::ptr_eq(reference(a), reference(b)));
    assert!(!Arc::ptr_eq(reference(a), reference(c)));
    let cloned = first.clone();
    let DensityFunction::Add(d, _) = &cloned else {
        panic!()
    };
    assert!(Arc::ptr_eq(reference(a), reference(d)));
    let inline = parse_json(r#"{"type":"minecraft:add","argument1":{"type":"minecraft:cache_once","argument":2},"argument2":{"type":"minecraft:cache_once","argument":2}}"#).unwrap();
    let DensityFunction::Add(a, b) = inline else {
        panic!()
    };
    assert!(matches!(*a, DensityFunction::CacheOnce(_)));
    assert!(matches!(*b, DensityFunction::CacheOnce(_)));
    assert!(!std::ptr::eq(a.as_ref(), b.as_ref()));
}

#[test]
fn reference_errors_are_reported_and_do_not_poison_resolution() {
    let mut assets = BTreeMap::from([
        ("density_function/a.json", r#""b""#),
        ("density_function/b.json", r#""a""#),
        ("density_function/self.json", r#""self""#),
        ("density_function/broken.json", "{"),
        ("density_function/good.json", "3.5"),
    ]);
    let mut load = |path: &str| {
        assets
            .get(path)
            .map(|s| s.to_string())
            .ok_or_else(|| format!("missing {path}"))
    };
    let mut parser = FunctionParser::new(&mut load);
    for (input, message) in [
        (r#""a""#, "a -> b -> a"),
        (r#""self""#, "self -> self"),
        (r#""missing""#, "missing density_function/missing.json"),
        (r#""broken""#, "unterminated string"),
    ] {
        assert!(
            parser.parse(input).unwrap_err().contains(message),
            "{input}"
        );
        assert!(parser.resolving.is_empty());
    }
    assert_eq!(
        parser
            .parse(r#""good""#)
            .unwrap()
            .evaluate(0., 0., 0., &EvalContext::default()),
        3.5
    );
    drop(parser);
    assets.insert("density_function/a.json", "7");
    let mut load = |path: &str| Ok(assets[path].to_owned());
    let fresh = FunctionParser::new(&mut load).parse(r#""a""#).unwrap();
    assert_eq!(fresh.evaluate(0., 0., 0., &EvalContext::default()), 7.0);
}

fn expand(value: &Value) -> Value {
    match value {
        Value::String(name) => {
            let path = format!(
                "density_function/{}.json",
                name.strip_prefix("minecraft:").unwrap_or(name)
            );
            crate::assets::bundled()
                .get(&path)
                .map(expand)
                .unwrap_or_else(|| value.clone())
        }
        Value::Array(values) => values.iter().map(expand).collect(),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    let value = if matches!(key.as_str(), "type" | "noise" | "rarity_value_mapper")
                    {
                        value.clone()
                    } else {
                        expand(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn nodes(f: &DensityFunction, unique: bool, seen: &mut HashSet<usize>) -> usize {
    use DensityFunction as D;
    if let D::Reference(a) = f {
        if unique && !seen.insert(Arc::as_ptr(a) as usize) {
            return 0;
        }
        return nodes(a, unique, seen);
    }
    let children: Vec<&D> = match f {
        D::Add(a, b) | D::Mul(a, b) | D::Min(a, b) | D::Max(a, b) => vec![a, b],
        D::Clamp(a, _, _)
        | D::Abs(a)
        | D::Square(a)
        | D::Cube(a)
        | D::Squeeze(a)
        | D::Interpolated(a)
        | D::BlendDensity(a)
        | D::Cache2d(a)
        | D::CacheAllInCell(a)
        | D::FlatCache(a)
        | D::CacheOnce(a)
        | D::BlendOffset(a)
        | D::BlendAlpha(a)
        | D::NoOp(a)
        | D::QuarterNegative(a)
        | D::HalfNegative(a)
        | D::Invert(a) => vec![a],
        D::Lerp { alpha, min, max } => vec![alpha, min, max],
        D::Range {
            input,
            when_in,
            when_out,
            ..
        } => vec![input, when_in, when_out],
        D::Spline { coordinate, points } => std::iter::once(coordinate.as_ref())
            .chain(points.iter().map(|p| p.1.as_ref()))
            .collect(),
        D::WeirdScaledSampler { input, .. } => vec![input],
        D::IntervalSelect {
            input, functions, ..
        } => std::iter::once(input.as_ref())
            .chain(functions.iter().map(|f| f.as_ref()))
            .collect(),
        D::ShiftedNoise {
            shift_x,
            shift_y,
            shift_z,
            ..
        } => vec![shift_x, shift_y, shift_z],
        D::FindTopSurface {
            density,
            upper_bound,
            ..
        } => vec![density, upper_bound],
        _ => vec![],
    };
    1 + children
        .into_iter()
        .map(|child| nodes(child, unique, seen))
        .sum::<usize>()
}

#[test]
fn overworld_named_graph_inventory_and_asset_load_counts() {
    let settings = crate::assets::load("noise_settings/overworld.json").unwrap();
    for field in ["final_density", "preliminary_surface_level", "depth"] {
        let mut loads = BTreeMap::<String, usize>::new();
        let mut load = |path: &str| {
            *loads.entry(path.to_owned()).or_default() += 1;
            crate::assets::load(path).map(|v| v.to_string())
        };
        let f = FunctionParser::new(&mut load)
            .parse(&settings["noise_router"][field].to_string())
            .unwrap();
        assert!(loads.values().all(|&count| count == 1));
        let expanded = nodes(&f, false, &mut HashSet::new());
        let shared = nodes(&f, true, &mut HashSet::new());
        let old = parse_json(&expand(&settings["noise_router"][field]).to_string()).unwrap();
        assert_eq!(expanded, nodes(&old, true, &mut HashSet::new()));
        assert!(shared < expanded, "{field}: {shared}/{expanded}");
        println!("DENSITY_SHARING_INVENTORY {field}: expanded_nodes={expanded}, shared_nodes={shared}, asset_loads={}", loads.len());
    }
}

#[test]
fn named_dag_matches_expanded_arithmetic_bits_across_phases_seeds_and_contexts() {
    let settings = crate::assets::load("noise_settings/overworld.json").unwrap();
    for field in ["final_density", "preliminary_surface_level", "depth"] {
        let input = &settings["noise_router"][field];
        let shared = parse_json(&input.to_string()).unwrap();
        let expanded = parse_json(&expand(input).to_string()).unwrap();
        for seed in [0, -7, 1234, i64::MIN, 0] {
            for mode in [
                EvaluationMode::Raw,
                EvaluationMode::NoiseChunk,
                EvaluationMode::NoiseChunkMaterial,
            ] {
                for cells in [4, 1, 4] {
                    let ctx = EvalContext {
                        seed,
                        mode,
                        ..Default::default()
                    }
                    .with_noise_bounds(-16, 32, cells);
                    clear_density_caches();
                    for repeat in 0..2 {
                        for (x, y, z) in [
                            (-17., -55., 31.),
                            (-1., 17., -3.),
                            (0., 0., 0.),
                            (7., 63., 9.),
                            (-13., 43., 35.),
                        ] {
                            assert_eq!(
                                shared.evaluate(x, y, z, &ctx).to_bits(),
                                expanded.evaluate(x, y, z, &ctx).to_bits(),
                                "{field}, {seed}, {mode:?}, {cells}, {repeat}, {x},{y},{z}"
                            );
                        }
                    }
                    clear_density_caches();
                    assert_eq!(density_cache_entries(), [0; 5]);
                }
            }
        }
    }
}

#[test]
fn shared_named_markers_keep_seed_phase_geometry_and_blender_scopes() {
    let definition = json!({"type": "minecraft:cache_once", "argument": {
        "type": "minecraft:interpolated", "argument": {
            "type": "minecraft:blend_density", "argument": {
                "type": "minecraft:noise", "noise": "minecraft:temperature",
                "xz_scale": 0.25, "y_scale": 0.5
            }
        }
    }});
    let mut load = |_: &str| Ok(definition.to_string());
    let shared = FunctionParser::new(&mut load)
        .parse(r#"{"type":"minecraft:add","argument1":"a","argument2":"a"}"#)
        .unwrap();
    let expanded = parse_json(
        &json!({"type": "minecraft:add", "argument1": definition, "argument2": definition})
            .to_string(),
    )
    .unwrap();
    for geometry in [4, 8, 4] {
        // Production scopes clear when bounds, cell geometry or Blender change.
        clear_density_caches();
        for seed in [0, -7, i64::MIN, 0] {
            for mode in [
                EvaluationMode::Raw,
                EvaluationMode::NoiseChunk,
                EvaluationMode::NoiseChunkMaterial,
                EvaluationMode::NoiseChunk,
            ] {
                let ctx = EvalContext {
                    seed,
                    mode,
                    cell_width: geometry,
                    cell_height: 12 - geometry,
                    blend_density: |x, y, z, value| value + f64::from(x - y + z) * 0.25,
                    ..Default::default()
                }
                .with_noise_bounds(-16, 32, 1);
                for x in -17..=-9 {
                    let (x, y, z) = (f64::from(x), 17.0, 35.0);
                    assert_eq!(
                        shared.evaluate(x, y, z, &ctx).to_bits(),
                        expanded.evaluate(x, y, z, &ctx).to_bits(),
                        "{geometry}, {seed}, {mode:?}, {x}"
                    );
                }
            }
        }
        clear_density_caches();
        assert_eq!(density_cache_entries(), [0; 5]);
    }
}

#[test]
fn native_holder_identity_and_raw_seeded_density_bits() {
    let fixture: Value =
        serde_json::from_str(include_str!("../data/density_sharing_26_1.json")).unwrap();
    assert_eq!(
        fixture["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(fixture["protocol"], 775);
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(case["holders_same"], true);
        assert_eq!(case["values_same"], true);
        let f = parse_json(&case["input"].to_string()).unwrap();
        let DensityFunction::Add(a, b) = &f else {
            panic!()
        };
        assert!(Arc::ptr_eq(reference(a), reference(b)));
        for seeded in case["seeded"].as_array().unwrap() {
            assert_eq!(seeded["mapped_arguments_same"], true);
            assert_eq!(seeded["wrapped_arguments_same"], true);
            let ctx = EvalContext {
                seed: seeded["seed"].as_i64().unwrap(),
                mode: EvaluationMode::Raw,
                ..Default::default()
            };
            for point in seeded["points"].as_array().unwrap() {
                let p = &point["pos"];
                let value = f.evaluate(
                    p[0].as_f64().unwrap(),
                    p[1].as_f64().unwrap(),
                    p[2].as_f64().unwrap(),
                    &ctx,
                );
                assert_eq!(
                    format!("{:016x}", value.to_bits()),
                    point["bits"].as_str().unwrap(),
                    "{}: {point}",
                    case["name"]
                );
            }
        }
    }
}

#[test]
fn cloned_references_survive_owner_drop_and_copy_on_write_mutation() {
    let mut load = |_: &str| Ok("2".to_owned());
    let first = FunctionParser::new(&mut load).parse(r#""a""#).unwrap();
    let mut cloned = first.clone();
    assert!(Arc::ptr_eq(reference(&first), reference(&cloned)));
    let DensityFunction::Reference(a) = &mut cloned else {
        panic!()
    };
    *Arc::make_mut(a) = DensityFunction::Constant(9.0);
    assert_eq!(first.evaluate(0., 0., 0., &EvalContext::default()), 2.0);
    drop(first);
    assert_eq!(cloned.evaluate(0., 0., 0., &EvalContext::default()), 9.0);
    let input = json!({"type": "minecraft:add", "argument1": "a", "argument2": "a"});
    let mut load = |_: &str| Ok("3".to_owned());
    let later = FunctionParser::new(&mut load)
        .parse(&input.to_string())
        .unwrap();
    assert_eq!(later.evaluate(0., 0., 0., &EvalContext::default()), 6.0);
}
