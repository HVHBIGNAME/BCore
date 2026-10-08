//! Integration of lazy placement, base feature kernels and cave feature delegates.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::tree_world::{self, EffectWorld, SharedTreeEffects};
use crate::base_features::BaseFeatures;
use crate::block_predicate::{catalog, kind, FeatureEnvironment, FeatureResult};
use crate::dripstone::CaveRandomState;
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::ore::OreWorld;
use crate::placement::{self, ConfiguredFeatureDispatcher, PlacementProgram};
use crate::simplex::WorldgenRandom;
use crate::tick_request::TickRequest;

pub(super) enum Outcome {
    Complete(bool),
    Unavailable(String),
}

/// Region biome IDs belong to BCore's wire/save registry, not the native catalog.
/// Resolve resource identities once. This unit environment represents untouched
/// pre-light storage (native sky=15, block=0), not a height-based approximation.
/// Use `with_light` when the world already has initialized neighbouring sections.
pub struct GenerationEnvironment;

pub struct GenerationLightEnvironment<'a> {
    light: &'a crate::lighting::GenerationLight,
    missing: Option<&'a str>,
}

impl GenerationEnvironment {
    pub fn with_light(light: &crate::lighting::GenerationLight) -> GenerationLightEnvironment<'_> {
        GenerationLightEnvironment {
            light,
            missing: None,
        }
    }

    pub(super) fn with_light_or_missing<'a>(
        light: &'a crate::lighting::GenerationLight,
        missing: Option<&'a str>,
    ) -> GenerationLightEnvironment<'a> {
        GenerationLightEnvironment { light, missing }
    }
}

impl GenerationLightEnvironment<'_> {
    fn available(&self) -> FeatureResult<()> {
        self.missing.map_or(Ok(()), |reason| {
            Err(FeatureError::MissingData(reason.into()))
        })
    }
}

impl FeatureEnvironment for GenerationEnvironment {
    fn raw_brightness(&self, _world: &dyn FeatureWorld, _pos: Pos) -> FeatureResult<i32> {
        // Native WorldGenRegion.getSkyDarken() is 0, and unallocated native sky
        // storage returns 15 regardless of terrain and of block emission.
        Ok(15)
    }

    fn should_freeze_with_edge(
        &self,
        world: &dyn FeatureWorld,
        pos: Pos,
        must_be_at_edge: bool,
    ) -> FeatureResult<bool> {
        self.should_freeze_in_biome(world, world.feature_biome(pos), pos, must_be_at_edge)
    }

    fn should_freeze_in_biome(
        &self,
        world: &dyn FeatureWorld,
        biome: u32,
        pos: Pos,
        must_be_at_edge: bool,
    ) -> FeatureResult<bool> {
        crate::lighting::should_freeze(world, biome, pos, must_be_at_edge, 0)
    }

    fn should_snow(&self, world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
        crate::lighting::should_snow(world, world.feature_biome(pos), pos, 0)
    }

    fn biome_has_feature(&self, biome: u32, feature: &str) -> FeatureResult<bool> {
        static NATIVE_IDS: OnceLock<FeatureResult<BTreeMap<u32, u32>>> = OnceLock::new();
        let ids = NATIVE_IDS
            .get_or_init(|| {
                let native = catalog().documents["biome_ids"]
                    .as_object()
                    .ok_or_else(|| {
                        FeatureError::MissingData("native feature biome registry".into())
                    })?;
                native
                    .iter()
                    .map(|(name, id)| {
                        let wire = crate::biome::id(name).ok_or_else(|| {
                            FeatureError::MissingData(format!("BCore biome identity {name}"))
                        })?;
                        let native = id
                            .as_u64()
                            .and_then(|id| u32::try_from(id).ok())
                            .ok_or_else(|| {
                                FeatureError::InvalidConfig(format!("native biome ID for {name}"))
                            })?;
                        Ok((wire, native))
                    })
                    .collect()
            })
            .as_ref()
            .map_err(|error| error.clone())?;
        let native = ids.get(&biome).copied().ok_or_else(|| {
            FeatureError::MissingData(format!(
                "native feature biome for BCore wire/save ID {biome}"
            ))
        })?;
        catalog().biome_has_feature(native, feature)
    }
}

impl FeatureEnvironment for GenerationLightEnvironment<'_> {
    fn raw_brightness(&self, _world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<i32> {
        self.available()?;
        Ok(self.light.max_local_raw_brightness(pos))
    }

    fn biome_has_feature(&self, biome: u32, feature: &str) -> FeatureResult<bool> {
        GenerationEnvironment.biome_has_feature(biome, feature)
    }

    fn should_freeze_with_edge(
        &self,
        world: &dyn FeatureWorld,
        pos: Pos,
        must_be_at_edge: bool,
    ) -> FeatureResult<bool> {
        self.should_freeze_in_biome(world, world.feature_biome(pos), pos, must_be_at_edge)
    }

    fn should_freeze_in_biome(
        &self,
        world: &dyn FeatureWorld,
        biome: u32,
        pos: Pos,
        must_be_at_edge: bool,
    ) -> FeatureResult<bool> {
        self.available()?;
        crate::lighting::should_freeze(
            world,
            biome,
            pos,
            must_be_at_edge,
            self.light.block_brightness(pos),
        )
    }

    fn should_snow(&self, world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
        self.available()?;
        crate::lighting::should_snow(
            world,
            world.feature_biome(pos),
            pos,
            self.light.block_brightness(pos),
        )
    }
}

pub(super) fn place_named(
    name: &str,
    world: &mut crate::region::FeatureRegion,
    random: &mut WorldgenRandom,
    source: crate::ChunkPos,
    cave_random: &mut CaveRandomState,
) -> Result<Outcome, String> {
    place_named_with_environment(
        name,
        world,
        random,
        source,
        cave_random,
        &GenerationEnvironment,
    )
}

pub(super) fn place_named_with_environment(
    name: &str,
    world: &mut crate::region::FeatureRegion,
    random: &mut WorldgenRandom,
    source: crate::ChunkPos,
    cave_random: &mut CaveRandomState,
    environment: &dyn FeatureEnvironment,
) -> Result<Outcome, String> {
    let program = match programs()
        .get(name)
        .expect("sorted top-level placed feature")
    {
        Ok(program) => program,
        Err(error) => return Ok(Outcome::Unavailable(error.to_string())),
    };
    let world_seed = world.world_seed();
    let mut world = EffectWorld::new(world);
    let mut dispatcher = BaseFeatures::with_dispatcher(AdditionalFeatures {
        cave_random,
        tree_effects: world.effects.clone(),
        world_seed,
    });
    program
        .place(
            Some(name),
            &mut world,
            random,
            (source.x * 16, crate::MIN_Y, source.z * 16),
            environment,
            &mut dispatcher,
        )
        .map(Outcome::Complete)
        .map_err(|error| error.to_string())
}

/// Synchronous jigsaw FeaturePoolElement callback. Nested placement has no
/// top-level feature identity; biome filters retain native nested semantics.
/// Gaussian state and the feature RNG belong to the entire source task.
pub(super) fn place_structure_feature(
    name: &str,
    world: &mut crate::region::FeatureRegion,
    random: &mut WorldgenRandom,
    origin: Pos,
    cave_random: &mut CaveRandomState,
) -> FeatureResult<bool> {
    place_placed(
        &Value::String(name.into()),
        world,
        random,
        origin,
        cave_random,
        &GenerationEnvironment,
    )
}

pub(super) fn place_placed(
    reference: &Value,
    world: &mut crate::region::FeatureRegion,
    random: &mut WorldgenRandom,
    origin: Pos,
    cave_random: &mut CaveRandomState,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    check_nested(reference, 0)?;
    let world_seed = world.world_seed();
    let mut world = EffectWorld::new(world);
    let mut dispatcher = BaseFeatures::with_dispatcher(AdditionalFeatures {
        cave_random,
        tree_effects: world.effects.clone(),
        world_seed,
    });
    placement::place_nested(
        reference,
        &mut world,
        random,
        origin,
        environment,
        &mut dispatcher,
    )
}

/// Direct configured-feature callback with the same guarded world/effect and
/// caller-owned RNG contracts as nested placement.
pub(super) fn place_configured(
    document: &Value,
    name: Option<&str>,
    world: &mut crate::region::FeatureRegion,
    random: &mut WorldgenRandom,
    origin: Pos,
    cave_random: &mut CaveRandomState,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    check_configured(document, name, 0)?;
    let world_seed = world.world_seed();
    let mut world = EffectWorld::new(world);
    let mut dispatcher = BaseFeatures::with_dispatcher(AdditionalFeatures {
        cave_random,
        tree_effects: world.effects.clone(),
        world_seed,
    });
    dispatcher.place_configured(document, name, &mut world, random, origin, environment)
}

fn programs() -> &'static BTreeMap<String, FeatureResult<PlacementProgram>> {
    static PROGRAMS: OnceLock<BTreeMap<String, FeatureResult<PlacementProgram>>> = OnceLock::new();
    PROGRAMS.get_or_init(|| {
        let mut programs = BTreeMap::new();
        for step in 0..11 {
            for name in
                (0..).map_while(|index| crate::feature_sorter::sorter().feature_name(step, index))
            {
                let program = catalog()
                    .placed(name)
                    .and_then(PlacementProgram::parse)
                    .and_then(|program| {
                        check_configured(
                            &program.configured,
                            program.configured_name.as_deref(),
                            0,
                        )?;
                        Ok(program)
                    });
                programs.insert(name.into(), program);
            }
        }
        programs
    })
}

// Identify missing kernels before consuming a placed stream. A missing kernel
// is coverage, not a successful no-op merely because its sampled origins failed.
fn check_configured(document: &Value, name: Option<&str>, depth: usize) -> FeatureResult<()> {
    if depth > 64 {
        return Err(FeatureError::InvalidConfig(
            "cyclic/deep configured feature references".into(),
        ));
    }
    let config = &document["config"];
    match kind(document)? {
        "simple_block" | "block_column" | "disk" | "spring_feature" | "lake" | "seagrass"
        | "pointed_dripstone" | "dripstone_cluster" | "large_dripstone" | "sculk_patch"
        | "desert_well" => Ok(()),
        "geode" => crate::geode::check_configured("geode", config),
        "fossil" => crate::fossil::check_configured(config),
        "underwater_magma"
        | "bamboo"
        | "vines"
        | "freeze_top_layer"
        | "kelp"
        | "sea_pickle"
        | "block_pile"
        | "huge_brown_mushroom"
        | "huge_red_mushroom"
        | "coral_tree"
        | "coral_claw"
        | "coral_mushroom"
        | "block_blob"
        | "blue_ice"
        | "spike"
        | "iceberg" => crate::misc_features::supports_configured(document).and_then(|supported| {
            if supported {
                Ok(())
            } else {
                Err(FeatureError::Unsupported("misc kernel capability".into()))
            }
        }),
        "tree" | "fallen_tree" => tree_world::configured_name(document, name).map(|_| ()),
        "multiface_growth"
            if matches!(
                config["block"].as_str(),
                Some("minecraft:glow_lichen" | "minecraft:sculk_vein")
            ) =>
        {
            Ok(())
        }
        "root_system" => check_nested(&config["feature"], depth + 1),
        "vegetation_patch" | "waterlogged_vegetation_patch" => {
            check_nested(&config["vegetation_feature"], depth + 1)
        }
        "random_boolean_selector" => {
            check_nested(&config["feature_true"], depth + 1)?;
            check_nested(&config["feature_false"], depth + 1)
        }
        "simple_random_selector" => {
            for child in crate::block_predicate::array(&config["features"])? {
                check_nested(child, depth + 1)?;
            }
            Ok(())
        }
        "random_selector" => {
            for child in crate::block_predicate::array(&config["features"])? {
                check_nested(&child["feature"], depth + 1)?;
            }
            check_nested(&config["default"], depth + 1)
        }
        feature_type => Err(FeatureError::Unsupported(format!(
            "configured {feature_type} has no integrated kernel"
        ))),
    }
}

fn check_nested(reference: &Value, depth: usize) -> FeatureResult<()> {
    let document = match reference.as_str() {
        Some(name) => catalog().placed(name)?,
        None => reference,
    };
    let program = PlacementProgram::parse(document)?;
    check_configured(
        &program.configured,
        program.configured_name.as_deref(),
        depth,
    )
}

struct AdditionalFeatures<'a> {
    cave_random: &'a mut CaveRandomState,
    tree_effects: SharedTreeEffects,
    world_seed: i64,
}

impl ConfiguredFeatureDispatcher for AdditionalFeatures<'_> {
    fn next_gaussian(&mut self, random: &mut WorldgenRandom) -> FeatureResult<f64> {
        Ok(self.cave_random.next_gaussian(random))
    }

    fn place_configured(
        &mut self,
        document: &Value,
        name: Option<&str>,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        origin: Pos,
        environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        let feature_type = kind(document)?;
        let config = &document["config"];
        match feature_type {
            "desert_well" => crate::desert_well::place(world, random, origin),
            "fossil" => {
                let effects = self.tree_effects.clone();
                crate::fossil::place(
                    config,
                    world,
                    random,
                    origin,
                    &mut |world, positions, flags| {
                        tree_world::update_template_shapes(world, effects.clone(), positions, flags)
                    },
                )
            }
            "geode" => crate::geode::place_configured_with(
                feature_type,
                config,
                world,
                random,
                origin,
                self.world_seed,
                environment,
                self,
            ),
            "underwater_magma"
            | "bamboo"
            | "vines"
            | "freeze_top_layer"
            | "kelp"
            | "sea_pickle"
            | "block_pile"
            | "huge_brown_mushroom"
            | "huge_red_mushroom"
            | "coral_tree"
            | "coral_claw"
            | "coral_mushroom"
            | "block_blob"
            | "blue_ice"
            | "spike"
            | "iceberg" => crate::misc_features::place_configured_with(
                document,
                world,
                random,
                origin,
                environment,
                self,
            ),
            "tree" | "fallen_tree" => tree_world::place(
                document,
                name,
                world,
                random,
                origin,
                self.tree_effects.clone(),
            ),
            "pointed_dripstone" | "dripstone_cluster" | "large_dripstone" => {
                crate::dripstone::place_configured(
                    feature_type,
                    config,
                    &mut WorldView(world),
                    random,
                    origin,
                    self.cave_random,
                )
            }
            "sculk_patch" => crate::sculk::place(
                world,
                random,
                origin,
                &crate::sculk::Config::from_json(feature_type, config)?,
            ),
            "multiface_growth" if config["block"] == "minecraft:sculk_vein" => crate::sculk::place(
                world,
                random,
                origin,
                &crate::sculk::Config::from_json(feature_type, config)?,
            ),
            "root_system"
            | "vegetation_patch"
            | "waterlogged_vegetation_patch"
            | "multiface_growth" => crate::lush_caves::place_configured_with(
                feature_type,
                config,
                &mut WorldView(world),
                random,
                origin,
                self.cave_random,
                &mut NestedFeatures {
                    environment,
                    tree_effects: self.tree_effects.clone(),
                    world_seed: self.world_seed,
                },
            ),
            other => Err(FeatureError::Unsupported(format!(
                "configured {} ({other})",
                name.unwrap_or("<inline>")
            ))),
        }
    }
}

struct NestedFeatures<'a> {
    environment: &'a dyn FeatureEnvironment,
    tree_effects: SharedTreeEffects,
    world_seed: i64,
}

impl<W: FeatureWorld> crate::lush_caves::PlacedFeatureDelegate<W> for NestedFeatures<'_> {
    fn place(
        &mut self,
        placed: &Value,
        world: &mut W,
        random: &mut WorldgenRandom,
        origin: Pos,
        cave_random: &mut CaveRandomState,
    ) -> FeatureResult<bool> {
        placement::place_nested(
            placed,
            world,
            random,
            origin,
            self.environment,
            &mut BaseFeatures::with_dispatcher(AdditionalFeatures {
                cave_random,
                tree_effects: self.tree_effects.clone(),
                world_seed: self.world_seed,
            }),
        )
    }
}

/// Sized forwarding view for kernels accepting `impl FeatureWorld`. It adds no
/// state, RNG or caching; all reads and effects go through the guarded region.
struct WorldView<'a>(&'a mut dyn FeatureWorld);
impl OreWorld for WorldView<'_> {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.0.ocean_floor_wg(x, z)
    }
    fn get_block(&self, pos: Pos) -> Option<u32> {
        self.0.get_block(pos)
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        self.0.set_block(pos, state)
    }
}
impl FeatureWorld for WorldView<'_> {
    fn feature_biome(&self, pos: Pos) -> u32 {
        self.0.feature_biome(pos)
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.0.feature_height(kind, x, z)
    }
    fn can_write_feature(&self, pos: Pos) -> bool {
        self.0.can_write_feature(pos)
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        self.0.set_feature_block(pos, state, flags)
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        self.0.mark_feature_postprocessing(pos);
    }
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        self.0.schedule_feature_tick(request)
    }
    fn set_feature_brushable_loot(
        &mut self,
        pos: Pos,
        table: &str,
        seed: i64,
    ) -> FeatureResult<bool> {
        self.0.set_feature_brushable_loot(pos, table, seed)
    }
}

#[cfg(test)]
#[path = "biome_bridge_tests.rs"]
mod biome_bridge_tests;

#[cfg(test)]
#[path = "lighting_environment_tests.rs"]
mod lighting_environment_tests;

#[cfg(test)]
#[path = "fossil_tests.rs"]
mod fossil_tests;

#[cfg(test)]
#[path = "desert_well_tests.rs"]
mod desert_well_tests;
