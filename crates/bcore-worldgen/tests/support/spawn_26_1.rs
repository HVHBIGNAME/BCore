//! Constructor inputs captured independently from historical finalized mobs.
use bcore_worldgen::feature_world::FeatureHeightmap;
use bcore_worldgen::spawn::{BiomeSpawns, MobKind, SpawnError, SpawnResult, SpawnTag, SpawnerData};
use serde_json::Value;
use std::sync::OnceLock;

fn catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let catalog: Value = serde_json::from_str(include_str!(
            "../../data/generation_spawn_assets_26_1_v2.json"
        ))
        .unwrap();
        assert_eq!(catalog["minecraft"], "26.1");
        assert_eq!(catalog["protocol"], 775);
        assert_eq!(
            catalog["provenance"]["jar_sha256"],
            "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
        );
        catalog
    })
}

pub fn constructor_template(kind: MobKind) -> SpawnResult<SpawnTag> {
    SpawnTag::from_native_json(&catalog()["constructors"][kind.name()]["entity"]["typed_nbt"])
}

pub fn biome_spawns(biome: u32) -> SpawnResult<BiomeSpawns> {
    let name = bcore_worldgen::biome::name(biome);
    let row = catalog()["catalog"]["biomes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].as_str().unwrap().strip_prefix("minecraft:") == Some(name))
        .ok_or_else(|| SpawnError::MissingData(format!("historical biome {name}")))?;
    Ok(BiomeSpawns {
        probability: row["probability"].as_f64().unwrap() as f32,
        groups: row["spawns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| SpawnerData {
                kind: MobKind::from_name(row["type"].as_str().unwrap()).unwrap(),
                weight: row["weight"].as_i64().unwrap().try_into().unwrap(),
                min_count: row["min"].as_i64().unwrap().try_into().unwrap(),
                max_count: row["max"].as_i64().unwrap().try_into().unwrap(),
            })
            .collect(),
    })
}

pub fn heightmap(kind: MobKind) -> SpawnResult<FeatureHeightmap> {
    let row = catalog()["catalog"]["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == kind.name())
        .ok_or_else(|| SpawnError::MissingData(format!("historical spawn type {}", kind.name())))?;
    match row["heightmap"].as_str().unwrap() {
        "MOTION_BLOCKING" => Ok(FeatureHeightmap::MotionBlocking),
        "MOTION_BLOCKING_NO_LEAVES" => Ok(FeatureHeightmap::MotionBlockingNoLeaves),
        other => Err(SpawnError::MissingData(format!(
            "historical heightmap {other}"
        ))),
    }
}
