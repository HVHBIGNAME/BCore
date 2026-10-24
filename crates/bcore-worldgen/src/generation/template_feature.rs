//! Native TemplateFeature selection, centering and caller-owned placement RNG.
use serde_json::Value;

use super::tree_world::{self, SharedTreeEffects};
use crate::block_predicate::{array, invalid, string, FeatureResult};
use crate::feature_world::{FeatureWorld, Pos};
use crate::simplex::WorldgenRandom;
use crate::structure::template::{add, transform_block, Mirror, PlacementSettings, Rotation};
use crate::structure::template_pool::StructureAssets;

struct Entry {
    template: String,
    rotations: Vec<Rotation>,
    weight: usize,
}

struct Config {
    entries: Vec<Entry>,
    weight: usize,
}

impl Config {
    fn parse(value: &Value) -> FeatureResult<Self> {
        let mut entries = Vec::new();
        let mut weight: usize = 0;
        for value in array(&value["templates"])? {
            let data = &value["data"];
            let template = string(&data["id"])?;
            let rotations = match data.get("rotations") {
                None => Rotation::ALL.to_vec(),
                Some(rotations) => array(rotations)?
                    .iter()
                    .map(|rotation| {
                        Ok(match string(rotation)? {
                            "none" => Rotation::None,
                            "clockwise_90" => Rotation::Clockwise90,
                            "180" => Rotation::Clockwise180,
                            "counterclockwise_90" => Rotation::Counterclockwise90,
                            other => return Err(invalid(format!("template rotation {other}"))),
                        })
                    })
                    .collect::<FeatureResult<Vec<_>>>()?,
            };
            let entry_weight = crate::dripstone::integer(&value["weight"], 0, i32::MAX)? as usize;
            weight = weight
                .checked_add(entry_weight)
                .filter(|&sum| sum <= i32::MAX as usize)
                .ok_or_else(|| invalid("template total weight"))?;
            entries.push(Entry {
                template: template.into(),
                rotations,
                weight: entry_weight,
            });
        }
        Ok(Self { entries, weight })
    }

    fn select(&self, random: &mut WorldgenRandom) -> FeatureResult<&Entry> {
        if self.weight == 0 {
            return Err(invalid("empty template selection"));
        }
        let mut index = random.next_int(self.weight);
        for entry in &self.entries {
            if index < entry.weight {
                return Ok(entry);
            }
            index -= entry.weight;
        }
        unreachable!("validated native template weights")
    }
}

pub(super) fn check_configured(value: &Value) -> FeatureResult<()> {
    let config = Config::parse(value)?;
    for entry in config.entries {
        StructureAssets::bundled().template(&entry.template)?;
    }
    Ok(())
}

pub(super) fn place(
    value: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    world_seed: i64,
    effects: SharedTreeEffects,
) -> FeatureResult<bool> {
    let config = Config::parse(value)?;
    let entry = config.select(random)?;
    if entry.rotations.is_empty() {
        return Err(invalid("empty template rotations"));
    }
    let rotation = entry.rotations[random.next_int(entry.rotations.len())];
    let assets = StructureAssets::bundled();
    let template = assets.template(&entry.template)?;
    let offset = transform_block(
        (-template.size.0 / 2, 0, -template.size.2 / 2),
        Mirror::None,
        rotation,
        (0, 0, 0),
    );
    let settings = PlacementSettings {
        rotation,
        known_shape: false,
        flags: 3,
        world_seed,
        ..Default::default()
    };
    template
        .place_feature(
            world,
            random,
            &assets.blocks,
            add(origin, offset),
            &settings,
            &mut |world, effect| world.apply_feature_template_effect(effect),
            &mut |world, positions, flags| {
                tree_world::update_template_shapes(world, effects.clone(), positions, flags)
            },
        )
        .map(|result| result.placed)
}

#[cfg(test)]
#[path = "template_feature_tests.rs"]
mod tests;
