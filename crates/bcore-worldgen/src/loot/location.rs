use super::{json, LootContext, LootError, Result};
use serde_json::Value;
use std::collections::BTreeSet;

/// A lookup observes the loaded world at the supplied block position. None means
/// the chunk is unloaded; unavailable biome data must be reported as an error.
pub trait LootWorld {
    fn biome_if_loaded(&self, pos: [i32; 3]) -> Result<Option<u32>>;
}

#[derive(Clone, Copy)]
pub struct LootLocation<'a> {
    pub origin: Option<[f64; 3]>,
    pub world: &'a dyn LootWorld,
}

impl std::fmt::Debug for LootLocation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LootLocation")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub(super) struct LocationCheck {
    offset: [i32; 3],
    biomes: Option<BTreeSet<u32>>,
}

impl LocationCheck {
    pub fn compile(value: &Value, path: &str) -> Result<Self> {
        json::fields(
            value,
            &["condition", "predicate", "offsetX", "offsetY", "offsetZ"],
            path,
        )?;
        let mut offset = [0; 3];
        for (index, name) in ["offsetX", "offsetY", "offsetZ"].into_iter().enumerate() {
            if let Some(value) = value.get(name) {
                offset[index] = json::int(value, &format!("{path}.{name}"))?;
            }
        }
        let biomes = if let Some(predicate) = value.get("predicate") {
            json::fields(predicate, &["biomes"], &format!("{path}.predicate"))?;
            predicate
                .get("biomes")
                .map(|value| {
                    crate::block_predicate::catalog()
                        .biome_holder_set(value)
                        .map_err(|error| json::invalid(path, error.to_string()))
                })
                .transpose()?
        } else {
            None
        };
        Ok(Self { offset, biomes })
    }

    pub fn test(&self, context: LootContext<'_>) -> Result<bool> {
        let location = context
            .location
            .ok_or(LootError::UnsupportedContext("loot location lookup"))?;
        let Some(origin) = location.origin else {
            return Ok(false);
        };
        let Some(biomes) = &self.biomes else {
            return Ok(true);
        };
        let pos = std::array::from_fn(|index| floor(origin[index] + f64::from(self.offset[index])));
        let Some(biome) = location.world.biome_if_loaded(pos)? else {
            return Ok(false);
        };
        if biome as usize >= bcore_registry::BIOME_COUNT {
            return Err(LootError::UnsupportedContext("unregistered location biome"));
        }
        Ok(biomes.contains(&biome))
    }
}

fn floor(value: f64) -> i32 {
    let integer = value as i32;
    if value < f64::from(integer) {
        integer.wrapping_sub(1)
    } else {
        integer
    }
}
