//! Resolve preserved configured/placed inputs before invoking current kernels.
use serde_json::Value;
use std::sync::OnceLock;

pub fn catalog() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        let data: Value =
            serde_json::from_str(include_str!("../../data/base_feature_catalog_26_1.json"))
                .unwrap();
        assert_eq!(data["minecraft"], "26.1");
        assert_eq!(
            data["jar_sha256"],
            "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
        );
        assert_eq!(data["state_count"], 29873);
        data
    })
}

fn resource(directory: &str, name: &str) -> &'static Value {
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    catalog()[directory]
        .get(&name)
        .unwrap_or_else(|| panic!("missing historical {directory}/{name}"))
}

pub fn placed(value: &Value) -> Value {
    expand_placed(value, 0)
}

pub fn configured(value: &Value) -> Value {
    expand_configured(value, 0)
}

pub fn target_biome(historical: u32) -> u32 {
    let name = catalog()["biome_ids"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, id)| id.as_u64() == Some(u64::from(historical)))
        .expect("historical biome resource identity")
        .0;
    bcore_worldgen::biome::id(name).expect("shared biome resource identity")
}

fn expand_placed(value: &Value, depth: usize) -> Value {
    assert!(depth < 64, "cyclic historical placed-feature references");
    let value = match value.as_str() {
        Some(name) => resource("placed_feature", name),
        None => value,
    };
    let mut result = value.clone();
    result["feature"] = expand_configured(&value["feature"], depth + 1);
    result
}

fn expand_configured(value: &Value, depth: usize) -> Value {
    assert!(
        depth < 64,
        "cyclic historical configured-feature references"
    );
    let value = match value.as_str() {
        Some(name) => resource("configured_feature", name),
        None => value,
    };
    let mut result = value.clone();
    let config = &mut result["config"];
    match value["type"].as_str().unwrap() {
        "minecraft:sequence" | "minecraft:simple_random_selector" => {
            for feature in config["features"].as_array_mut().unwrap() {
                *feature = expand_placed(feature, depth + 1);
            }
        }
        "minecraft:random_selector" => {
            for feature in config["features"].as_array_mut().unwrap() {
                feature["feature"] = expand_placed(&feature["feature"], depth + 1);
            }
            config["default"] = expand_placed(&config["default"], depth + 1);
        }
        "minecraft:weighted_random_selector" => {
            for feature in config["features"].as_array_mut().unwrap() {
                feature["data"] = expand_placed(&feature["data"], depth + 1);
            }
        }
        "minecraft:random_boolean_selector" => {
            for key in ["feature_true", "feature_false"] {
                config[key] = expand_placed(&config[key], depth + 1);
            }
        }
        "minecraft:root_system" => config["feature"] = expand_placed(&config["feature"], depth + 1),
        "minecraft:vegetation_patch" | "minecraft:waterlogged_vegetation_patch" => {
            config["vegetation_feature"] = expand_placed(&config["vegetation_feature"], depth + 1);
        }
        _ => {}
    }
    result
}
