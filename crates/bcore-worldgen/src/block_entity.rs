//! Generated block-entity data, before gameplay or loot unpacking.
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnerMob {
    Skeleton,
    Zombie,
    Spider,
    CaveSpider,
}

impl SpawnerMob {
    pub fn name(self) -> &'static str {
        match self {
            Self::Skeleton => "minecraft:skeleton",
            Self::Zombie => "minecraft:zombie",
            Self::Spider => "minecraft:spider",
            Self::CaveSpider => "minecraft:cave_spider",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockEntity {
    DungeonChest {
        loot_seed: i64,
    },
    Spawner {
        mob: SpawnerMob,
    },
    /// Generated nectarless bees, in native occupant order. Native codecs accept
    /// any i32 age and do not impose the gameplay admission limit on saved lists.
    Beehive {
        ticks_in_hive: Vec<i32>,
    },
}

impl BlockEntity {
    pub fn type_id(&self) -> u32 {
        match self {
            Self::DungeonChest { .. } => 1,
            Self::Spawner { .. } => 9,
            Self::Beehive { .. } => 34,
        }
    }

    pub fn matches_state(&self, state: u32) -> bool {
        match self {
            Self::DungeonChest { .. } => crate::dungeon::state_flags(state) & 16 != 0,
            Self::Spawner { .. } => crate::dungeon::state_flags(state) & 32 != 0,
            Self::Beehive { .. } => (21768..21816).contains(&state),
        }
    }

    pub fn update_data(&self) -> Value {
        match self {
            Self::DungeonChest { .. } | Self::Beehive { .. } => json!({}),
            Self::Spawner { mob } => json!({
                "Delay": 20, "MaxNearbyEntities": 6, "MaxSpawnDelay": 800, "MinSpawnDelay": 200,
                "RequiredPlayerRange": 16, "SpawnCount": 4, "SpawnRange": 4,
                "SpawnData": {"entity": {"id": mob.name()}}
            }),
        }
    }

    pub fn full_data(&self, (x, y, z): (i32, i32, i32)) -> Value {
        let mut data = self.update_data();
        let name = match self {
            Self::DungeonChest { loot_seed } => {
                data["LootTable"] = json!("minecraft:chests/simple_dungeon");
                if *loot_seed != 0 {
                    data["LootTableSeed"] = json!(loot_seed);
                }
                "minecraft:chest"
            }
            Self::Spawner { .. } => {
                data["SpawnPotentials"] = json!([]);
                "minecraft:mob_spawner"
            }
            Self::Beehive { ticks_in_hive } => {
                data["bees"] = ticks_in_hive
                    .iter()
                    .map(|ticks| {
                        json!({
                            "ticks_in_hive": ticks,
                            "entity_data": {"id": "minecraft:bee"},
                            "min_ticks_in_hive": 600,
                        })
                    })
                    .collect();
                "minecraft:beehive"
            }
        };
        data["components"] = json!({});
        data["id"] = json!(name);
        data["x"] = json!(x);
        data["y"] = json!(y);
        data["z"] = json!(z);
        data
    }
}
