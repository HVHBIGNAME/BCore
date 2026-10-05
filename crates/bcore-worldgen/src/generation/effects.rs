//! Typed generated block entities not yet represented by the gameplay enum.
use std::sync::OnceLock;

use crate::feature_world::{FeatureError, Pos};
use crate::structure::template::{
    nbt_codec_int, Mirror, Nbt, Rotation, TemplateBlockEntity, TemplateEntity,
};
use crate::structure::template_pool::StructureAssets;

/// The typed tree is `[NBT tag ID, payload]`, preserving TAG_Long and other
/// distinctions that ordinary JSON cannot encode. Persistence must retain it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FeatureBlockEntity {
    pub type_id: u32,
    pub full_data: serde_json::Value,
    pub update_data: serde_json::Value,
    pub typed_data: serde_json::Value,
    pub valid_states: [u32; 2],
    /// Original post-processor/loadWithComponents payload, preserving all NBT
    /// widths. None keeps the previous generated-sculk serialization valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_load_data: Option<serde_json::Value>,
    /// Native update-tag codec, separate from the full saved data. Lists use
    /// {element_type, values}, so empty-list element types survive persistence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_update_data: Option<serde_json::Value>,
}

impl FeatureBlockEntity {
    pub fn matches_state(&self, state: u32) -> bool {
        (self.valid_states[0]..self.valid_states[1]).contains(&state)
    }

    /// Validate the generated sculk defaults, including NBT widths and ownership.
    /// Live block-entity mutations need their own codec when they are implemented.
    pub fn valid_for(&self, state: u32, pos: Pos) -> bool {
        if !self.matches_state(state) {
            return false;
        }
        if let Some(load) = &self.template_load_data {
            return Nbt::from_typed_json(load)
                .and_then(|load| {
                    TemplateBlockEntity::from_load(
                        &StructureAssets::bundled().blocks,
                        state,
                        pos,
                        load,
                    )
                })
                .and_then(|entity| Self::from_template(&entity))
                .is_ok_and(|expected| expected == *self);
        }
        sculk_block_entity(state, pos).is_ok_and(|expected| expected.as_ref() == Some(self))
    }

    pub fn from_template(entity: &TemplateBlockEntity) -> Result<Self, FeatureError> {
        let blocks = &StructureAssets::bundled().blocks;
        let definition =
            crate::block_predicate::catalog().definition(&blocks.state(entity.state)?.name)?;
        let full = entity.full_data();
        let update = template_update(entity, &full)?;
        Ok(Self {
            type_id: entity.type_id,
            full_data: full.to_json(),
            update_data: update.to_json(),
            typed_data: full.typed_json(),
            valid_states: [definition.first, definition.first + definition.count],
            template_load_data: Some(entity.load_data.typed_json()),
            typed_update_data: Some(update.typed_json()),
        })
    }

    /// Physical NBT consumed by the protocol's typed anonymous-NBT encoder.
    pub fn update_nbt(&self) -> Result<Nbt, FeatureError> {
        if let Some(typed) = &self.typed_update_data {
            return Nbt::from_typed_json(typed);
        }
        if self
            .update_data
            .as_object()
            .is_some_and(|value| value.is_empty())
        {
            return Ok(Nbt::empty_compound());
        }
        Err(FeatureError::MissingData(
            "typed block-entity update tag".into(),
        ))
    }

    /// Full saved NBT, decoding the previous sculk logical-list representation
    /// separately from new physical template lists. No serialization is changed.
    pub fn full_nbt(&self) -> Result<Nbt, FeatureError> {
        if self.template_load_data.is_some() {
            Nbt::from_typed_json(&self.typed_data)
        } else {
            Nbt::from_logical_typed_json(&self.typed_data)
        }
    }
}

fn template_update(entity: &TemplateBlockEntity, full: &Nbt) -> Result<Nbt, FeatureError> {
    let id = entity.id.as_str();
    let fields: &[&str] = match id {
        // These classes inherit BlockEntity.getUpdateTag's empty compound, as
        // independently captured from the pinned JAR (including loaded NBT).
        "minecraft:furnace"
        | "minecraft:chest"
        | "minecraft:trapped_chest"
        | "minecraft:ender_chest"
        | "minecraft:dispenser"
        | "minecraft:dropper"
        | "minecraft:brewing_stand"
        | "minecraft:hopper"
        | "minecraft:barrel"
        | "minecraft:smoker"
        | "minecraft:blast_furnace"
        | "minecraft:lectern"
        | "minecraft:bell"
        | "minecraft:bed"
        | "minecraft:comparator"
        | "minecraft:daylight_detector"
        | "minecraft:enchanting_table"
        | "minecraft:beehive"
        | "minecraft:sculk_sensor"
        | "minecraft:sculk_catalyst"
        | "minecraft:sculk_shrieker"
        | "minecraft:calibrated_sculk_sensor" => &[],
        "minecraft:sign" | "minecraft:hanging_sign" => &["front_text", "back_text", "is_waxed"],
        "minecraft:campfire" => &["Items"],
        // DecoratedPotBlockEntity uses saveCustomOnly: loot/item and ordered
        // decorations, without metadata or the full save's components compound.
        "minecraft:decorated_pot" => &["sherds", "LootTable", "LootTableSeed", "item"],
        "minecraft:vault" => &["shared_data"],
        "minecraft:trial_spawner" => {
            let mut data = std::collections::BTreeMap::new();
            let state = StructureAssets::bundled().blocks.state(entity.state)?;
            if state
                .properties
                .get("trial_spawner_state")
                .map(String::as_str)
                == Some("active")
            {
                data.insert(
                    "next_mob_spawns_at".into(),
                    full.get("next_mob_spawns_at")
                        .cloned()
                        .unwrap_or(Nbt::Long(0)),
                );
            }
            if let Some(spawn) = full.get("spawn_data") {
                data.insert("spawn_data".into(), spawn.clone());
            }
            return Ok(Nbt::Compound(data));
        }
        "minecraft:skull" => &["profile", "note_block_sound", "custom_name"],
        "minecraft:banner" => {
            let mut data = full.compound()?.clone();
            for key in ["id", "x", "y", "z"] {
                data.remove(key);
            }
            return Ok(Nbt::Compound(data));
        }
        "minecraft:brushable_block" => {
            let mut data = std::collections::BTreeMap::new();
            if let Some(direction) = entity
                .load_data
                .get("hit_direction")
                .and_then(nbt_codec_int)
            {
                // Direction.from3DDataValue uses abs(remainder), not floorMod.
                data.insert(
                    "hit_direction".into(),
                    Nbt::Byte((direction % 6).abs() as i8),
                );
            }
            if let Some(item) = full.get("item") {
                data.insert("item".into(), item.clone());
            }
            return Ok(Nbt::Compound(data));
        }
        _ => {
            return Err(FeatureError::Unsupported(format!(
                "native template update codec {id}"
            )))
        }
    };
    Ok(Nbt::Compound(
        fields
            .iter()
            .filter_map(|&key| full.get(key).cloned().map(|v| (key.to_owned(), v)))
            .collect(),
    ))
}

/// A pending native STRUCTURE entity factory/finalization request, not an
/// approximation of a spawned villager/animal. Identity allocation, subclass
/// orientation, equipment, passengers and mob finalization are still required.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StructureEntityRequest {
    pub source_chunk: [i32; 2],
    pub position_bits: [u64; 3],
    pub block_pos: [i32; 3],
    pub typed_data: serde_json::Value,
    pub finalize: bool,
    pub rotation: Rotation,
    pub mirror: Mirror,
}

impl StructureEntityRequest {
    pub(crate) fn from_template(entity: TemplateEntity, source: crate::ChunkPos) -> Self {
        Self {
            source_chunk: [source.x, source.z],
            position_bits: entity.pos.map(f64::to_bits),
            block_pos: [entity.block_pos.0, entity.block_pos.1, entity.block_pos.2],
            typed_data: entity.nbt.typed_json(),
            finalize: entity.finalize,
            rotation: entity.rotation,
            mirror: entity.mirror,
        }
    }

    pub fn position(&self) -> [f64; 3] {
        self.position_bits.map(f64::from_bits)
    }

    pub fn valid_for(&self, owner: crate::ChunkPos) -> bool {
        let p = self.position();
        if !p.iter().all(|v| v.is_finite())
            || p[0].floor() < i32::MIN as f64
            || p[0].floor() > i32::MAX as f64
            || p[2].floor() < i32::MIN as f64
            || p[2].floor() > i32::MAX as f64
            || p[1] < crate::MIN_Y as f64
            || p[1] >= (crate::MAX_Y + 1) as f64
            || (p[0].floor() as i32 >> 4, p[2].floor() as i32 >> 4) != (owner.x, owner.z)
        {
            return false;
        }
        let Ok(nbt) = Nbt::from_typed_json(&self.typed_data) else {
            return false;
        };
        nbt.get("id")
            .and_then(Nbt::string)
            .is_some_and(|id| id.starts_with("minecraft:"))
            && nbt.get("UUID").is_none()
            && nbt.get("Pos")
                == Some(&Nbt::List {
                    element_type: 6,
                    values: p.into_iter().map(Nbt::Double).collect(),
                })
    }
}

pub(crate) fn sculk_block_entity(
    state: u32,
    pos: Pos,
) -> Result<Option<FeatureBlockEntity>, FeatureError> {
    static RANGES: OnceLock<[[u32; 2]; 3]> = OnceLock::new();
    let ranges = RANGES.get_or_init(|| {
        ["sculk_sensor", "sculk_catalyst", "sculk_shrieker"].map(|name| {
            let definition = crate::block_predicate::catalog()
                .definition(name)
                .expect("native sculk block definition");
            [definition.first, definition.first + definition.count]
        })
    });
    let Some(&valid_states) = ranges
        .iter()
        .find(|range| (range[0]..range[1]).contains(&state))
    else {
        return Ok(None);
    };
    let entity = crate::sculk::generated_block_entity(state, pos)?.ok_or_else(|| {
        FeatureError::MissingData(format!(
            "generated sculk block entity at {pos:?}, state {state}"
        ))
    })?;
    Ok(Some(FeatureBlockEntity {
        type_id: entity.type_id,
        full_data: entity.full_data,
        update_data: entity.update_data,
        typed_data: entity.typed_data,
        valid_states,
        template_load_data: None,
        typed_update_data: None,
    }))
}
