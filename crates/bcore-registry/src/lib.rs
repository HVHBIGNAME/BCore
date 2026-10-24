#![forbid(unsafe_code)]
//! Vanilla 26.2 registry identities shared by generation, protocol and persistence.
//!
//! Static registries are extracted from the authenticated server artifact. Dynamic
//! registry order is synchronized during configuration. State properties and names
//! identify blocks across versions; numeric IDs apply only to this contract.

mod generated;
pub use generated::*;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RegistryEntry {
    pub id: u32,
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub struct BlockState {
    pub id: u32,
    pub name: String,
    pub properties: BTreeMap<String, String>,
    pub default: bool,
}

#[derive(Debug, Deserialize)]
pub struct VanillaRegistries {
    pub minecraft: String,
    pub protocol: i32,
    pub data_version: i32,
    pub jar_sha256: String,
    pub registries: BTreeMap<String, Vec<RegistryEntry>>,
    pub states: Vec<BlockState>,
}

pub fn vanilla() -> &'static VanillaRegistries {
    static REGISTRIES: OnceLock<VanillaRegistries> = OnceLock::new();
    REGISTRIES.get_or_init(|| {
        let value: VanillaRegistries =
            serde_json::from_str(include_str!("../data/registries_26_2.json"))
                .expect("authenticated vanilla registry data");
        assert_eq!(value.minecraft, MC_VERSION);
        assert_eq!(value.protocol, PROTOCOL_VERSION);
        assert_eq!(value.data_version, DATA_VERSION);
        assert_eq!(value.jar_sha256, SERVER_JAR_SHA256);
        assert_eq!(value.states.len(), BLOCK_STATE_COUNT);
        for (id, state) in value.states.iter().enumerate() {
            assert_eq!(state.id as usize, id);
        }
        for entries in value.registries.values() {
            for (id, entry) in entries.iter().enumerate() {
                assert_eq!(entry.id as usize, id);
            }
        }
        assert_eq!(value.registries["BIOME"].len(), BIOME_COUNT);
        value
    })
}

pub fn block_state_name(id: u32) -> Option<&'static str> {
    vanilla()
        .states
        .get(id as usize)
        .map(|state| state.name.as_str())
}

pub fn registry_id(registry: &str, name: &str) -> Option<u32> {
    vanilla()
        .registries
        .get(registry)?
        .iter()
        .find(|entry| entry.key == name)
        .map(|entry| entry.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn registry_identity_matches_the_published_fingerprint() {
        let bytes = include_bytes!("../data/registries_26_2.json");
        assert_eq!(bytes.last(), Some(&b'\n'));
        let fingerprint: [u8; 32] = Sha256::digest(&bytes[..bytes.len() - 1]).into();
        assert_eq!(fingerprint, REGISTRY_FINGERPRINT);
        let data = vanilla();
        assert_eq!(
            data.states[block_state::COPPER_ORE as usize].name,
            "minecraft:copper_ore"
        );
        assert_eq!(
            data.states[block_state::DEEPSLATE as usize].properties["axis"],
            "y"
        );
        assert_eq!(
            registry_id("ENTITY_TYPE", "minecraft:zombie"),
            Some(entity_type::ZOMBIE)
        );
        assert_eq!(
            registry_id("ATTRIBUTE", "minecraft:movement_speed"),
            Some(attribute::MOVEMENT_SPEED)
        );
    }
}
