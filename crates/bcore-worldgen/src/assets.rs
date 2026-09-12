//! Extracted vanilla data travels with the binary, including in unit tests.
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub fn bundled() -> &'static BTreeMap<String, Value> {
    static ASSETS: OnceLock<BTreeMap<String, Value>> = OnceLock::new();
    ASSETS.get_or_init(|| {
        serde_json::from_str(include_str!("../data/vanilla_worldgen.json"))
            .expect("valid bundled vanilla worldgen data")
    })
}

pub fn load(path: &str) -> Result<Value, String> {
    if let Some(root) = std::env::var_os("BCORE_DATAPACK") {
        let root = std::path::PathBuf::from(root);
        let file = if path == "biome_parameters/overworld.json" {
            root.join("../datagen/reports/biome_parameters/minecraft/overworld.json")
        } else if path == "biome_registry.json" {
            return bundled().get(path).cloned().ok_or_else(|| path.to_owned());
        } else {
            root.join("data/minecraft/worldgen").join(path)
        };
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
    } else {
        bundled()
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing vanilla asset: {path}"))
    }
}
