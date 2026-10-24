//! Matched native-style requests; holder watches precede returned-object saves.
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::sync::Arc;

use bcore_core::ChunkPos;
use bcore_worldgen::generation::request::NativeChunkRequests;
use bcore_worldgen::generation::{ChunkAccessKind, ChunkProgress, ChunkStatus, StageState};
use bcore_worldgen::structure::template::Nbt;
use bcore_worldgen::tick_queue::{PreparedTickQueues, SavedTick};
use bcore_worldgen::{GeneratedChunk, GenerationWorld};
use serde_json::{json, Value};

const SCHEMA: u64 = 3;
const OBSERVATION_CONTRACT: &str = "native_get_chunk_returned_object_v3";

fn status(name: &str) -> Result<ChunkStatus, Box<dyn Error>> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    ChunkStatus::ALL
        .into_iter()
        .find(|s| s.name().strip_prefix("minecraft:").unwrap_or(s.name()) == name)
        .ok_or_else(|| format!("unsupported status {name}").into())
}

fn pos(value: &Value) -> Result<ChunkPos, Box<dyn Error>> {
    if value.as_array().is_none_or(|values| values.len() != 2) {
        return Err("position must contain exactly x,z".into());
    }
    Ok(ChunkPos::new(
        i32::try_from(value[0].as_i64().ok_or("x must be an integer")?)?,
        i32::try_from(value[1].as_i64().ok_or("z must be an integer")?)?,
    ))
}

fn request(spec: &Value) -> Result<(ChunkPos, ChunkStatus, bool), Box<dyn Error>> {
    let create = match spec.get("create") {
        Some(value) => value.as_bool().ok_or("create must be a boolean")?,
        None => true, // HistoryProbe's scripted getChunk calls always pass true.
    };
    Ok((
        pos(&spec["pos"])?,
        status(spec["status"].as_str().ok_or("missing status")?)?,
        create,
    ))
}

fn validate_config(config: &Value) -> Result<(i64, i64), Box<dyn Error>> {
    if config["schema"].as_u64() != Some(SCHEMA)
        || config["observation_contract"].as_str() != Some(OBSERVATION_CONTRACT)
    {
        return Err("rebuild the replay plan for returned-object observations (schema 3)".into());
    }
    let game_time: i64 = config["game_time"]
        .as_str()
        .ok_or("game_time must be an exact i64 string")?
        .parse()?;
    if game_time != 0 {
        return Err("this fresh-world replay requires the recorded game time to be zero".into());
    }
    let seed = config["seed"]
        .as_str()
        .ok_or("seed must be an exact i64 string")?
        .parse()?;
    let bootstrap = config["bootstrap_requests"]
        .as_array()
        .ok_or("bootstrap_requests must be an explicit array")?;
    match config["bootstrap"].as_str() {
        Some("before_spawn") if bootstrap.is_empty() => (),
        Some("native") if config["bootstrap_replayed"].as_bool() == Some(true) => (),
        _ => return Err("bootstrap requires its recorded synchronous request history".into()),
    }
    for field in [
        "native_provenance_sha256",
        "native_events_sha256",
        "native_config_sha256",
        "native_registry_sha256",
    ] {
        let digest = config[field].as_str().ok_or("missing native provenance")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!("invalid {field}").into());
        }
    }
    let requests = config["requests"]
        .as_array()
        .ok_or("requests must be an array")?;
    for spec in bootstrap.iter().chain(requests) {
        let (position, _, _) = request(spec)?;
        NativeChunkRequests::validate_position(position)?;
    }
    let mut watches = BTreeSet::new();
    for watch in config["watch"].as_array().ok_or("watch must be an array")? {
        let position = pos(watch)?;
        NativeChunkRequests::validate_position(position)?;
        if !watches.insert((position.x, position.z)) {
            return Err("duplicate watch position".into());
        }
    }
    if let Some(trace) = config.get("feature_trace") {
        NativeChunkRequests::validate_position(pos(&trace["source"])?)?;
        if trace["feature"].as_str().is_none_or(str::is_empty) {
            return Err("trace feature must be a nonempty string".into());
        }
    }
    Ok((seed, game_time))
}

fn snapshot(chunk: &GeneratedChunk) -> Value {
    let mut biomes = Vec::with_capacity(1536);
    for y in (-64..320).step_by(4) {
        for z in (0..16).step_by(4) {
            for x in (0..16).step_by(4) {
                biomes.push(format!(
                    "minecraft:{}",
                    bcore_worldgen::biome::name(chunk.noise_biome_at(x, y, z))
                ));
            }
        }
    }
    let heights: Vec<_> = (0..16)
        .flat_map(|z| (0..16).map(move |x| chunk.height_at(x, z)))
        .collect();
    let worldgen_heightmaps = chunk.worldgen_heightmaps().map(|maps| {
        json!({
            "WORLD_SURFACE_WG": maps.world_surface,
            "OCEAN_FLOOR_WG": maps.ocean_floor,
        })
    });
    let runtime_heightmaps = chunk.runtime_heightmaps().map(|maps| {
        json!({
            "WORLD_SURFACE": maps.world_surface,
            "OCEAN_FLOOR": maps.ocean_floor,
            "MOTION_BLOCKING": maps.motion_blocking,
            "MOTION_BLOCKING_NO_LEAVES": maps.motion_blocking_no_leaves,
        })
    });
    let mut block_entities: Vec<_> = chunk
        .block_entities()
        .iter()
        .map(|(&(x, y, z), entity)| {
            let p = (chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32);
            json!({"pos": p, "type": entity.type_id(), "nbt": entity.full_data(p)})
        })
        .collect();
    block_entities.extend(chunk.feature_block_entities().iter().map(|(&(x, y, z), entity)| {
        let p = (chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32);
        json!({"pos": p, "type": entity.type_id, "nbt": entity.full_data, "typed_nbt": entity.typed_data})
    }));
    block_entities.extend(chunk.pending_block_entities().iter().map(|(&(x, y, z), entity)| {
        let p = (chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32);
        let nbt = entity.full_nbt().expect("validated pending proto NBT");
        json!({"pos": p, "pending": true, "nbt": nbt.to_json(), "typed_nbt": entity.typed_data})
    }));
    let entities: Vec<_> = chunk
        .entities()
        .iter()
        .map(|e| json!({"pos": e.position(), "type": e.type_id(), "nbt": e.data()}))
        .collect();
    json!({
        "pos": [chunk.pos.x, chunk.pos.z], "min_y": -64, "states": chunk.states(),
        "biomes": biomes, "heights": heights, "worldgen_heightmaps": worldgen_heightmaps,
        "runtime_heightmaps": runtime_heightmaps,
        "light": chunk.light(), "structure_entities": chunk.structure_entities(),
        "structures": chunk.structures().native_data(chunk.pos),
        "block_entities": block_entities, "entities": entities,
        "tick_requests": chunk.tick_requests(),
        "postprocessing": chunk.postprocessing_positions(),
    })
}

fn saved_ticks(ticks: &[SavedTick<u32>], fluid: bool) -> Result<Vec<Value>, Box<dyn Error>> {
    let catalog = bcore_worldgen::block_predicate::catalog();
    ticks
        .iter()
        .map(|tick| {
            let name = if fluid {
                catalog
                    .fluids
                    .iter()
                    .find_map(|(name, &id)| (id == tick.target).then_some(name.as_str()))
                    .ok_or_else(|| format!("unknown saved fluid tick type {}", tick.target))?
            } else {
                catalog.block(tick.target)?.0
            };
            Ok(tick.save_data(name))
        })
        .collect()
}

fn saved_snapshot(
    chunk: &GeneratedChunk,
    tags: &BTreeMap<(usize, i32, usize), Nbt>,
    block_ticks: &[SavedTick<u32>],
    fluid_ticks: &[SavedTick<u32>],
) -> Result<Value, Box<dyn Error>> {
    let mut value = snapshot(chunk);
    value["block_entities"] = json!(tags
        .iter()
        .map(|(&(x, y, z), tag)| {
            let type_id = chunk
                .block_entities()
                .get(&(x, y, z))
                .map(|entity| entity.type_id())
                .or_else(|| {
                    chunk
                        .feature_block_entities()
                        .get(&(x, y, z))
                        .map(|entity| entity.type_id)
                });
            json!({
                "pos": [chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32],
                "type": type_id, "nbt": tag.to_json(), "typed_nbt": tag,
            })
        })
        .collect::<Vec<_>>());
    value["saved_ticks"] = json!({
        "block_ticks": saved_ticks(block_ticks, false)?,
        "fluid_ticks": saved_ticks(fluid_ticks, true)?,
    });
    Ok(value)
}

fn lifecycle(value: &mut Value, kind: ChunkAccessKind, persisted_status: ChunkStatus) {
    value["raw_object_kind"] = json!(format!("{kind:?}"));
    // HistoryProbe explicitly unwraps ImposterProtoChunk before serialization.
    let (boundary, class) = match kind {
        ChunkAccessKind::Proto => ("proto_save", "net.minecraft.world.level.chunk.ProtoChunk"),
        ChunkAccessKind::ImposterProto | ChunkAccessKind::Level => {
            ("level_save", "net.minecraft.world.level.chunk.LevelChunk")
        }
    };
    value["snapshot_boundary"] = json!(boundary);
    value["chunk_class"] = json!(class);
    value["persisted_status"] = json!(persisted_status.name());
    value["serialized_status"] = json!(persisted_status.name());
}

fn stages(progress: &ChunkProgress) -> Vec<Value> {
    ChunkStatus::ALL
        .into_iter()
        .map(|status| {
            let stage = progress.stage(status);
            json!({"status": status.name(), "state": format!("{:?}", stage.state),
                   "attempts": stage.attempts, "missing": stage.missing})
        })
        .collect()
}

// These observations deliberately follow the holder. They never request/pump
// native work, and cannot be used in place of a cached ReturnedChunk's save.
fn holder_snapshot(
    world: &GenerationWorld,
    position: ChunkPos,
    game_time: i64,
) -> Result<Option<Value>, Box<dyn Error>> {
    if world.chunk_access(position, ChunkStatus::Full)?.is_some() {
        let saved = world.prepare_chunk_save(position, game_time)?;
        let mut value = saved_snapshot(
            &saved.chunk,
            &saved.block_entity_tags,
            &saved.block_ticks,
            &saved.fluid_ticks,
        )?;
        lifecycle(&mut value, ChunkAccessKind::Level, ChunkStatus::Full);
        let progress = world.progress(position)?.ok_or("holder progress absent")?;
        value["implementation_stages"] = json!(stages(&progress));
        return Ok(Some(value));
    }
    if world.chunk_access(position, ChunkStatus::Empty)?.is_none() {
        return Ok(None);
    }
    let chunk = world
        .chunk_snapshot(position)?
        .ok_or("holder data absent")?;
    let mut value = snapshot(&chunk);
    let ticks = PreparedTickQueues::from_requests(chunk.tick_requests().iter().copied());
    value["saved_ticks"] = json!({
        "block_ticks": saved_ticks(&ticks.blocks.pack(game_time), false)?,
        "fluid_ticks": saved_ticks(&ticks.fluids.pack(game_time), true)?,
    });
    let progress = world.progress(position)?.ok_or("holder progress absent")?;
    let available = ChunkStatus::ALL
        .into_iter()
        .take_while(|&s| {
            matches!(
                progress.stage(s).state,
                StageState::Complete | StageState::Partial
            )
        })
        .last()
        .ok_or("holder has no available status")?;
    lifecycle(&mut value, ChunkAccessKind::Proto, available);
    value["implementation_stages"] = json!(stages(&progress));
    Ok(Some(value))
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--version" {
        println!(
            "{}",
            json!({"program": "native-history-replay", "schema": SCHEMA,
                   "observation_contract": OBSERVATION_CONTRACT, "feature_trace_version": 1})
        );
        return Ok(());
    }
    if args.len() != 3 {
        return Err("usage: native-history-replay <replay-config.json> <new-output.jsonl>".into());
    }
    let config: Value = serde_json::from_reader(BufReader::new(File::open(&args[1])?))?;
    let (seed, game_time) = validate_config(&config)?;
    let world = Arc::new(GenerationWorld::new(seed));
    let mut trace_out = if let Some(trace) = config.get("feature_trace") {
        world.trace_feature(
            pos(&trace["source"])?,
            trace["feature"].as_str().ok_or("trace feature")?,
        )?;
        Some(BufWriter::new(
            File::options()
                .write(true)
                .create_new(true)
                .open(std::path::Path::new(&args[2]).with_file_name("feature_traces.jsonl"))?,
        ))
    } else {
        None
    };
    let native_requests = NativeChunkRequests::new(world.clone())?;
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    let mut out = BufWriter::new(file);
    let mut sources = BTreeMap::new();
    let bootstrap = config["bootstrap_requests"].as_array().unwrap();
    let requests = config["requests"]
        .as_array()
        .ok_or("requests must be an array")?;
    for (index, spec) in bootstrap.iter().chain(requests.iter()).enumerate() {
        let (position, target, create) = request(spec)?;
        let returned = native_requests.get_chunk(position, target, create)?;
        let mut watches = Vec::new();
        let (observed, coverage) = if index < bootstrap.len() {
            // Native bootstrap records calls, not scripted save boundaries.
            let view = returned.as_ref().map(|object| object.view()).transpose()?;
            (Value::Null, view.map(|view| view.coverage))
        } else {
            for watch in config["watch"].as_array().unwrap() {
                let position = pos(watch)?;
                let mut observed = match holder_snapshot(&world, position, game_time)? {
                    Some(snapshot) => snapshot,
                    None => json!({"pos": [position.x, position.z], "absent": true}),
                };
                observed["snapshot_origin"] = json!("holder_latest");
                watches.push(observed);
            }
            // An OLD Proto cache hit must remain Proto even after a holder
            // watch saved its canonical LevelChunk. Never reselect by position.
            let saved = returned
                .as_ref()
                .map(|object| object.prepare_save(game_time))
                .transpose()?;
            let mut observed = if let Some(saved) = &saved {
                let mut value = saved_snapshot(
                    &saved.view.chunk,
                    &saved.block_entity_tags,
                    &saved.block_ticks,
                    &saved.fluid_ticks,
                )?;
                lifecycle(&mut value, saved.view.kind, saved.view.persisted_status);
                value
            } else {
                json!({"pos": [position.x, position.z], "absent": true})
            };
            observed["snapshot_origin"] = json!("returned_object");
            (observed, saved.map(|saved| saved.view.coverage))
        };
        let coverage = coverage.as_ref();
        if let Some(coverage) = coverage {
            for source in &coverage.feature_sources {
                sources.insert(source.sequence, json!({
                    "source": [source.source.x, source.source.z], "sequence": source.sequence,
                    "completed": source.completed.iter().map(|work| json!({
                        "step": work.step, "index": work.index, "feature": work.feature, "placed": work.placed,
                    })).collect::<Vec<_>>(),
                    "missing": source.missing.iter().map(|work| json!({
                        "step": work.step, "index": work.index, "feature": work.feature, "reason": work.reason,
                    })).collect::<Vec<_>>(),
                }));
            }
        }
        serde_json::to_writer(
            &mut out,
            &json!({
                "schema": SCHEMA, "observation_contract": OBSERVATION_CONTRACT,
                "request": index as i64 - bootstrap.len() as i64,
                "phase": if index < bootstrap.len() { "bootstrap" } else { "requests" },
                "spec": spec, "seed": config["seed"], "game_time": config["game_time"],
                "native_provenance_sha256": config["native_provenance_sha256"],
                "native_events_sha256": config["native_events_sha256"],
                "native_config_sha256": config["native_config_sha256"],
                "native_registry_sha256": config["native_registry_sha256"],
                "snapshot": observed, "watch": watches,
                "complete": coverage.map(|c| c.is_complete()),
                "coverage_status": coverage.map(|c| c.requested_status.name()),
                "stages": coverage.map(|c| stages(&c.target)),
                "feature_sources": sources.values().collect::<Vec<_>>(),
            }),
        )?;
        writeln!(out)?;
        out.flush()?;
        if let Some(out) = &mut trace_out {
            for trace in world.take_feature_traces()? {
                serde_json::to_writer(
                    &mut *out,
                    &json!({
                        "request": index as i64 - bootstrap.len() as i64,
                        "source": [trace.source.x, trace.source.z], "feature": trace.feature,
                        "boundary": trace.boundary, "next_i64": trace.next_i64,
                        "chunks": trace.chunks.iter().map(snapshot).collect::<Vec<_>>(),
                    }),
                )?;
                writeln!(out)?;
            }
            out.flush()?;
        }
        eprintln!(
            "request {index}: {position:?} {target}, complete={:?}",
            coverage.map(|c| c.is_complete())
        );
    }
    Ok(())
}
