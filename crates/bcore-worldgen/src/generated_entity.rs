//! Entity data produced by features, before simulation and loot unpacking.
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedEntity {
    ChestMinecart { block_pos: [i32; 3], loot_seed: i64 },
}

impl GeneratedEntity {
    pub fn block_pos(&self) -> [i32; 3] {
        match self {
            Self::ChestMinecart { block_pos, .. } => *block_pos,
        }
    }

    pub fn position(&self) -> [f64; 3] {
        self.block_pos().map(|v| f64::from(v) + 0.5)
    }

    pub fn type_id(&self) -> u32 {
        match self {
            Self::ChestMinecart { .. } => 25,
        }
    }

    /// Canonical generated NBT values. UUID is assigned independently of the
    /// placement RNG and is intentionally outside this deterministic payload.
    pub fn data(&self) -> Value {
        match self {
            Self::ChestMinecart { loot_seed, .. } => {
                let mut data = json!({
                    "id": "minecraft:chest_minecart", "Pos": self.position(),
                    "Motion": [0.0, 0.0, 0.0], "Rotation": [0.0, 0.0],
                    "fall_distance": 0.0, "Fire": 0, "Air": 300,
                    "OnGround": 0, "Invulnerable": 0, "PortalCooldown": 0,
                    "FlippedRotation": 0, "HasTicked": 1,
                    "LootTable": "minecraft:chests/abandoned_mineshaft"
                });
                if *loot_seed != 0 {
                    data["LootTableSeed"] = json!(loot_seed);
                }
                data
            }
        }
    }
}
