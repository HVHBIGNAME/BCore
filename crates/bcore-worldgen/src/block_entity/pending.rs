//! The proto-chunk's saved tags are not live block entities or update packets.
use std::collections::BTreeMap;

use crate::feature_world::{FeatureError, Pos};
use crate::generation::FeatureBlockEntity;
use crate::structure::template::{Nbt, TemplateBlockEntity};
use crate::structure::template_pool::StructureAssets;

use super::BlockEntity;

/// Native `ChunkAccess.pendingBlockEntities` data. The physical typed tree keeps
/// numeric widths and empty-list types intact until an actual lookup loads it.
/// `DUMMY` is a factory request with coordinates, never a network BE type.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingBlockEntity {
    pub typed_data: serde_json::Value,
}

/// The existing two runtime representations, produced only by materialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializedBlockEntity {
    Generated(BlockEntity),
    Feature(FeatureBlockEntity),
}

impl PendingBlockEntity {
    pub fn dummy((x, y, z): Pos) -> Self {
        Self::from_nbt(&Nbt::Compound(BTreeMap::from([
            ("id".into(), Nbt::String("DUMMY".into())),
            ("x".into(), Nbt::Int(x)),
            ("y".into(), Nbt::Int(y)),
            ("z".into(), Nbt::Int(z)),
        ])))
    }

    pub fn from_nbt(nbt: &Nbt) -> Self {
        Self {
            typed_data: nbt.typed_json(),
        }
    }

    /// Proto saving observes the tag as-is and does not instantiate anything.
    pub fn full_nbt(&self) -> Result<Nbt, FeatureError> {
        Nbt::from_typed_json(&self.typed_data)
    }

    pub fn valid_for(&self, state: u32, pos: Pos) -> bool {
        let Ok(nbt) = self.full_nbt() else {
            return false;
        };
        has_block_entity(state)
            && nbt
                .get("id")
                .and_then(Nbt::string)
                .is_some_and(|id| !id.is_empty())
            && [("x", pos.0), ("y", pos.1), ("z", pos.2)]
                .into_iter()
                .all(|(key, value)| nbt.get(key) == Some(&Nbt::Int(value)))
    }

    /// Native WorldGenRegion lookup: DUMMY invokes the state's factory; a saved
    /// tag goes through its load codec. On unsupported input the caller retains
    /// the pending tag. This does not convert a chunk or run gameplay callbacks.
    pub fn materialize(
        &self,
        state: u32,
        pos: Pos,
    ) -> Result<MaterializedBlockEntity, FeatureError> {
        if !self.valid_for(state, pos) {
            return Err(FeatureError::InvalidConfig(format!(
                "pending block entity at {pos:?} does not belong to state {state}"
            )));
        }
        let blocks = &StructureAssets::bundled().blocks;
        let expected = blocks.default_block_entity(state, pos)?.ok_or_else(|| {
            FeatureError::MissingData(format!("block entity factory for state {state}"))
        })?;
        let nbt = self.full_nbt()?;
        if nbt.get("id").and_then(Nbt::string) == Some("DUMMY") {
            if expected.id == "minecraft:beehive" {
                return Ok(MaterializedBlockEntity::Generated(BlockEntity::Beehive {
                    ticks_in_hive: Vec::new(),
                }));
            }
            if let Some(data) = crate::generation::sculk_block_entity(state, pos)? {
                return Ok(MaterializedBlockEntity::Feature(data));
            }
            return FeatureBlockEntity::from_template(&expected)
                .map(MaterializedBlockEntity::Feature);
        }
        if nbt.get("id").and_then(Nbt::string) != Some(expected.id.as_str()) {
            return Err(FeatureError::Unsupported(format!(
                "pending block entity id {:?} for {} at {pos:?}",
                nbt.get("id"),
                expected.id
            )));
        }
        let mut load = nbt.compound()?.clone();
        // Chunk save metadata is not passed through to a runtime update/save tag.
        for key in ["id", "x", "y", "z", "keepPacked"] {
            load.remove(key);
        }
        validate_saved_load(&expected, &load)?;
        let entity = TemplateBlockEntity::from_load(blocks, state, pos, Nbt::Compound(load))?;
        FeatureBlockEntity::from_template(&entity).map(MaterializedBlockEntity::Feature)
    }
}

/// Saved-holder hydration is not implemented in general. Admit the captured
/// generated forms (and the independently tested pot/brushable load codecs),
/// rather than laundering arbitrary saved NBT through a default-plus-JSON merge.
fn validate_saved_load(
    expected: &TemplateBlockEntity,
    load: &BTreeMap<String, Nbt>,
) -> Result<(), FeatureError> {
    if matches!(
        expected.id.as_str(),
        "minecraft:brushable_block" | "minecraft:decorated_pot"
    ) {
        return Ok(());
    }
    let defaults = expected.full_data();
    for (key, value) in load {
        let valid = if key == "components" {
            matches!(value, Nbt::Compound(fields) if fields.is_empty())
        } else {
            match expected.id.as_str() {
                "minecraft:chest" | "minecraft:dispenser" => match key.as_str() {
                    "LootTable" => value.string().is_some_and(|id| {
                        id.starts_with("minecraft:")
                            && id[10..].bytes().all(|c| {
                                c.is_ascii_lowercase() || c.is_ascii_digit() || b"/._-".contains(&c)
                            })
                            && id.len() > 10
                    }),
                    "LootTableSeed" => matches!(value, Nbt::Long(_)),
                    "Items" => {
                        matches!(value, Nbt::List { element_type: 0, values } if values.is_empty())
                    }
                    _ => false,
                },
                "minecraft:sculk_sensor"
                | "minecraft:sculk_shrieker"
                | "minecraft:sculk_catalyst"
                | "minecraft:calibrated_sculk_sensor" => match key.as_str() {
                    "last_vibration_frequency" | "warning_level" => {
                        defaults.get(key).is_some() && matches!(value, Nbt::Int(_))
                    }
                    "listener" | "cursors" => defaults.get(key) == Some(value),
                    _ => false,
                },
                "minecraft:beehive" if key == "bees" => generated_bees(value),
                "minecraft:comparator" => defaults.get(key) == Some(value),
                _ => false,
            }
        };
        if !valid {
            return Err(FeatureError::Unsupported(format!(
                "saved {} field {key}: runtime load codec is not verified",
                expected.id
            )));
        }
    }
    if !matches!(
        expected.id.as_str(),
        "minecraft:chest"
            | "minecraft:dispenser"
            | "minecraft:sculk_sensor"
            | "minecraft:sculk_shrieker"
            | "minecraft:sculk_catalyst"
            | "minecraft:calibrated_sculk_sensor"
            | "minecraft:beehive"
            | "minecraft:comparator"
    ) {
        return Err(FeatureError::Unsupported(format!(
            "saved {} materialization",
            expected.id
        )));
    }
    Ok(())
}

fn generated_bees(value: &Nbt) -> bool {
    let Nbt::List {
        element_type,
        values,
    } = value
    else {
        return false;
    };
    if values.is_empty() {
        return *element_type == 0;
    }
    *element_type == 10
        && values.iter().all(|bee| {
            let Some(Nbt::Int(ticks)) = bee.get("ticks_in_hive") else {
                return false;
            };
            bee == &Nbt::Compound(BTreeMap::from([
                (
                    "entity_data".into(),
                    Nbt::Compound(BTreeMap::from([(
                        "id".into(),
                        Nbt::String("minecraft:bee".into()),
                    )])),
                ),
                ("ticks_in_hive".into(), Nbt::Int(*ticks)),
                ("min_ticks_in_hive".into(), Nbt::Int(600)),
            ]))
        })
}

pub(crate) fn has_block_entity(state: u32) -> bool {
    StructureAssets::bundled()
        .blocks
        .flags(state)
        .is_ok_and(|flags| flags & 8 != 0)
}

impl crate::GeneratedChunk {
    /// Like ChunkAccess.setBlockEntityNbt, a pending tag cannot overwrite a live
    /// entity. A later proto write may replace another pending tag with DUMMY.
    pub fn set_pending_block_entity(
        &mut self,
        x: usize,
        y: i32,
        z: usize,
        data: PendingBlockEntity,
    ) -> bool {
        let Some(state) = self.get(x, y, z) else {
            return false;
        };
        let Some(pos) = self.block_entity_world_pos(x, y, z) else {
            return false;
        };
        if self.block_entities.contains_key(&(x, y, z))
            || self.feature_block_entities.contains_key(&(x, y, z))
            || !data.valid_for(state, pos)
        {
            return false;
        }
        self.pending_block_entities.insert((x, y, z), data);
        true
    }

    fn block_entity_world_pos(&self, x: usize, y: i32, z: usize) -> Option<Pos> {
        Some((
            self.pos
                .x
                .checked_mul(16)?
                .checked_add(i32::try_from(x).ok()?)?,
            y,
            self.pos
                .z
                .checked_mul(16)?
                .checked_add(i32::try_from(z).ok()?)?,
        ))
    }

    /// WorldGenRegion's getBlockEntity boundary. Unlike a direct proto map read,
    /// this can materialize a pending factory/load request. Missing tags do not
    /// synthesize an entity merely because the block supports one.
    pub fn materialize_block_entity(
        &mut self,
        x: usize,
        y: i32,
        z: usize,
    ) -> Result<bool, FeatureError> {
        let local = (x, y, z);
        if self.block_entities.contains_key(&local)
            || self.feature_block_entities.contains_key(&local)
        {
            return Ok(true);
        }
        let Some(pending) = self.pending_block_entities.get(&local) else {
            return Ok(false);
        };
        let state = self.get(x, y, z).expect("validated pending position");
        let pos = self
            .block_entity_world_pos(x, y, z)
            .expect("validated pending owner");
        let data = pending.materialize(state, pos)?;
        self.install_materialized_block_entity(local, data);
        Ok(true)
    }

    fn install_materialized_block_entity(
        &mut self,
        local: (usize, i32, usize),
        data: MaterializedBlockEntity,
    ) {
        match data {
            MaterializedBlockEntity::Generated(data) => {
                self.block_entities.insert(local, data);
            }
            MaterializedBlockEntity::Feature(data) => {
                self.feature_block_entities.insert(local, data);
            }
        }
        self.pending_block_entities.remove(&local);
    }

    /// Explicit BE-only materialization boundary for FULL/loaded consumers.
    /// It does not perform FULL conversion, postprocessing, ticks or WG-map
    /// retirement. Unsupported codecs leave the entire pending batch intact.
    pub fn materialize_block_entities(&mut self) -> Result<usize, FeatureError> {
        let mut loaded = Vec::with_capacity(self.pending_block_entities.len());
        for (&local, pending) in &self.pending_block_entities {
            let (x, y, z) = local;
            let state = self.get(x, y, z).expect("validated pending position");
            let pos = self
                .block_entity_world_pos(x, y, z)
                .expect("validated pending owner");
            loaded.push((local, pending.materialize(state, pos)?));
        }
        let count = loaded.len();
        for (local, data) in loaded {
            self.install_materialized_block_entity(local, data);
        }
        Ok(count)
    }
}
