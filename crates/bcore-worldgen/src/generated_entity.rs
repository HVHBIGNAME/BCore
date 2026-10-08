//! Entity data produced by features, before simulation and loot unpacking.
use serde_json::{json, Value};

mod mob;
pub use mob::GeneratedMob;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedEntity {
    ChestMinecart { block_pos: [i32; 3], loot_seed: i64 },
    Mob(Box<GeneratedMob>),
}

impl GeneratedEntity {
    pub fn block_pos(&self) -> [i32; 3] {
        match self {
            Self::ChestMinecart { block_pos, .. } => *block_pos,
            Self::Mob(mob) => mob.position().map(|v| v.floor() as i32),
        }
    }

    pub fn position(&self) -> [f64; 3] {
        match self {
            Self::ChestMinecart { .. } => self.block_pos().map(|v| f64::from(v) + 0.5),
            Self::Mob(mob) => mob.position(),
        }
    }

    pub fn type_id(&self) -> u32 {
        match self {
            Self::ChestMinecart { .. } => 25,
            Self::Mob(mob) => crate::spawn::spawn_type(mob.kind()).registry_id,
        }
    }

    pub fn valid_for(&self, owner: bcore_core::ChunkPos) -> bool {
        let [x, y, z] = self.block_pos();
        if x >> 4 != owner.x || z >> 4 != owner.z {
            return false;
        }
        match self {
            Self::ChestMinecart { .. } => (crate::MIN_Y..=crate::MAX_Y).contains(&y),
            // Entities can stand at the first free Y above the last block. Their
            // validated floating-point positions are not block-array indices.
            Self::Mob(_) => true,
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
            Self::Mob(mob) => mob.data(),
        }
    }
}
