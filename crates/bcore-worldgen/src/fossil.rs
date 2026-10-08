//! Pinned overworld FossilFeature: native templates, shared placement RNG and
//! retained OCEAN_FLOOR_WG, with shape callbacks between the bone/ore passes.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

use crate::block_predicate::{catalog, read_block, FeatureResult};
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::simplex::WorldgenRandom;
use crate::structure::processors::{process_block_infos, Processor};
use crate::structure::template::{
    BlockInfo, BoundingBox, Mirror, PlacementSettings, ProcessorRandom, Rotation, StructureTemplate,
};
use crate::structure::template_pool::StructureAssets;

struct Config {
    native: Value,
    fossils: Vec<String>,
    overlays: Vec<String>,
    fossil_processors: Vec<Processor>,
    overlay_processors: Vec<Processor>,
    max_empty_corners: usize,
}

struct Fossils {
    templates: BTreeMap<String, StructureTemplate>,
    configs: Vec<Config>,
}

fn fossils() -> &'static Fossils {
    static DATA: OnceLock<Fossils> = OnceLock::new();
    DATA.get_or_init(|| {
        #[derive(Deserialize)]
        struct NativeTemplate {
            size: Pos,
            palettes: Vec<Vec<BlockInfo>>,
        }
        #[derive(Deserialize)]
        struct Data {
            jar_sha256: String,
            templates: BTreeMap<String, NativeTemplate>,
        }
        let data: Data = serde_json::from_str(include_str!("../data/fossil_assets_26_1.json"))
            .expect("native fossil assets");
        assert_eq!(data.jar_sha256, StructureAssets::JAR_SHA256);
        let assets = StructureAssets::bundled();
        let templates: BTreeMap<_, _> = data
            .templates
            .into_iter()
            .map(|(name, template)| {
                // These templates contain only solid bone/coal states. There is
                // no template NBT, loot RNG, entity or liquid-container handoff.
                for info in template.palettes.iter().flatten() {
                    assert!(info.nbt.is_none());
                    assert!(matches!(
                        assets.blocks.state(info.state).unwrap().name.as_str(),
                        "minecraft:bone_block" | "minecraft:coal_ore"
                    ));
                }
                let template = StructureTemplate::new(
                    template.size,
                    template.palettes,
                    Vec::new(),
                    &assets.blocks,
                )
                .expect("validated native fossil template");
                (name, template)
            })
            .collect();
        let configs = ["fossil_coal", "fossil_diamonds"]
            .into_iter()
            .map(|name| {
                let native = catalog().configured(name).unwrap()["config"].clone();
                let fossils: Vec<String> =
                    serde_json::from_value(native["fossil_structures"].clone()).unwrap();
                let overlays: Vec<String> =
                    serde_json::from_value(native["overlay_structures"].clone()).unwrap();
                assert!(!fossils.is_empty() && fossils.len() == overlays.len());
                assert!(fossils
                    .iter()
                    .chain(&overlays)
                    .all(|s| templates.contains_key(s)));
                Config {
                    fossils,
                    overlays,
                    fossil_processors: assets
                        .resolve_processors(&native["fossil_processors"])
                        .unwrap(),
                    overlay_processors: assets
                        .resolve_processors(&native["overlay_processors"])
                        .unwrap(),
                    max_empty_corners: native["max_empty_corners_allowed"].as_u64().unwrap()
                        as usize,
                    native,
                }
            })
            .collect();
        Fossils { templates, configs }
    })
}

fn config(value: &Value) -> FeatureResult<&'static Config> {
    fossils()
        .configs
        .iter()
        .find(|config| config.native == *value)
        .ok_or_else(|| {
            FeatureError::Unsupported("non-native fossil configuration/templates".into())
        })
}

pub(crate) fn check_configured(value: &Value) -> FeatureResult<()> {
    config(value).map(|_| ())
}

pub(crate) fn place(
    value: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    update_shapes: &mut impl FnMut(&mut dyn FeatureWorld, &[Pos], i32) -> FeatureResult<()>,
) -> FeatureResult<bool> {
    let config = config(value)?;
    let rotation = Rotation::ALL[random.next_int(Rotation::ALL.len())];
    let index = random.next_int(config.fossils.len());
    let template = &fossils().templates[&config.fossils[index]];
    let (width, depth) = match rotation {
        Rotation::Clockwise90 | Rotation::Counterclockwise90 => (template.size.2, template.size.0),
        _ => (template.size.0, template.size.2),
    };
    let (x, z) = (origin.0 - width / 2, origin.2 - depth / 2);
    let mut surface = origin.1;
    for dx in 0..width {
        for dz in 0..depth {
            surface =
                surface.min(world.feature_height(FeatureHeightmap::OceanFloorWg, x + dx, z + dz));
        }
    }
    let y = (surface - 15 - random.next_int(10) as i32).max(crate::MIN_Y + 10);
    let local_bounds = template.bounding_box((0, 0, 0), rotation, Mirror::None, (0, 0, 0));
    // Native getZeroPositionWithTransform aligns the rotated template's minimum
    // corner with the centred footprint, including negative world coordinates.
    let zero = (x - local_bounds.min.0, y, z - local_bounds.min.2);
    let bounds = local_bounds.moved(zero);
    let mut empty = 0;
    for x in [bounds.min.0, bounds.max.0] {
        for y in [bounds.min.1, bounds.max.1] {
            for z in [bounds.min.2, bounds.max.2] {
                let state = read_block(world, (x, y, z))?;
                empty += usize::from(
                    crate::heightmap::is_air(state)
                        || matches!(
                            StructureAssets::bundled()
                                .blocks
                                .state(state)?
                                .name
                                .as_str(),
                            "minecraft:water" | "minecraft:lava"
                        ),
                );
            }
        }
    }
    if empty > config.max_empty_corners {
        return Ok(false);
    }
    let chunk = (origin.0 >> 4, origin.2 >> 4);
    let mut settings = PlacementSettings {
        rotation,
        clip: Some(BoundingBox {
            min: (chunk.0 * 16 - 16, crate::MIN_Y, chunk.1 * 16 - 16),
            max: (chunk.0 * 16 + 31, crate::MAX_Y, chunk.1 * 16 + 31),
        }),
        known_shape: false,
        flags: 260,
        ..Default::default()
    };
    for (name, processors) in [
        (&config.fossils[index], &config.fossil_processors),
        (&config.overlays[index], &config.overlay_processors),
    ] {
        settings.processors = processors.clone();
        place_template(
            &fossils().templates[name],
            world,
            random,
            zero,
            &settings,
            update_shapes,
        )?;
    }
    // FossilFeature ignores each template's placement boolean after admission.
    Ok(true)
}

fn place_template(
    template: &StructureTemplate,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    settings: &PlacementSettings,
    update_shapes: &mut impl FnMut(&mut dyn FeatureWorld, &[Pos], i32) -> FeatureResult<()>,
) -> FeatureResult<()> {
    let registry = &StructureAssets::bundled().blocks;
    // setRandom in native settings aliases the feature RNG. In particular even
    // a singleton palette consumes nextInt(1), and rot draws precede all writes.
    let mut settings_random = ProcessorRandom(Some(random));
    let palette = template.palette_with_random(origin, &mut settings_random)?;
    let infos = process_block_infos(
        world,
        registry,
        origin,
        origin,
        palette,
        settings,
        &mut settings_random,
    )?;
    let mut written = Vec::new();
    for info in infos {
        if settings.clip.is_some_and(|clip| !clip.contains(info.pos)) {
            continue;
        }
        // Native reads the previous fluid even though these solid fossil blocks
        // cannot receive waterlogging. Keep the world access before the write.
        read_block(world, info.pos)?;
        let state = registry.transform(info.state, settings.mirror, settings.rotation)?;
        if world.set_feature_block(info.pos, state, settings.flags) {
            written.push(info.pos);
        }
    }
    update_shapes(world, &written, settings.flags)
}
