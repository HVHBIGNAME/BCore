//! Diagnostic-only matched requests through one retained production GenerationWorld.
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use bcore_core::ChunkPos;
use bcore_worldgen::generation::ChunkStatus;
use bcore_worldgen::{GeneratedChunk, GenerationWorld};
use serde_json::{json, Value};

fn status(name: &str) -> Result<ChunkStatus, Box<dyn Error>> {
    ChunkStatus::ALL
        .into_iter()
        .find(|s| {
            s.name().trim_start_matches("minecraft:") == name.trim_start_matches("minecraft:")
        })
        .ok_or_else(|| format!("unsupported status {name}").into())
}

fn pos(value: &Value) -> Result<ChunkPos, Box<dyn Error>> {
    Ok(ChunkPos::new(
        i32::try_from(value[0].as_i64().ok_or("x must be an integer")?)?,
        i32::try_from(value[1].as_i64().ok_or("z must be an integer")?)?,
    ))
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
        "pos": [chunk.pos.x, chunk.pos.z], "states": chunk.states(),
        "biomes": biomes, "heights": heights, "worldgen_heightmaps": worldgen_heightmaps,
        "light": chunk.light(), "structure_entities": chunk.structure_entities(),
        "structures": chunk.structures().native_data(chunk.pos),
        "block_entities": block_entities, "entities": entities,
        "tick_requests": chunk.tick_requests(),
        "postprocessing": chunk.postprocessing_positions(),
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: native-history-replay <replay-config.json> <new-output.jsonl>".into());
    }
    let config: Value = serde_json::from_reader(BufReader::new(File::open(&args[1])?))?;
    let seed = config["seed"]
        .as_str()
        .ok_or("seed must be an exact i64 string")?
        .parse()?;
    if config["bootstrap"] != "before_spawn" && config["bootstrap_replayed"] != true {
        return Err("native bootstrap requires explicit replay of its recorded requests".into());
    }
    let world = GenerationWorld::new(seed);
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
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    let mut out = BufWriter::new(file);
    let mut sources = BTreeMap::new();
    let bootstrap = config["bootstrap_requests"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let requests = config["requests"]
        .as_array()
        .ok_or("requests must be an array")?;
    for (index, spec) in bootstrap.iter().chain(requests.iter()).enumerate() {
        let position = pos(&spec["pos"])?;
        let target = status(spec["status"].as_str().ok_or("missing status")?)?;
        let result = world.generate_to_status(position, target)?;
        let coverage = &result.coverage;
        let stages: Vec<_> = ChunkStatus::ALL.into_iter().map(|s| {
            let p = coverage.target.stage(s);
            json!({"status": s.name(), "state": format!("{:?}", p.state), "attempts": p.attempts, "missing": p.missing})
        }).collect();
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
        let mut watches = Vec::new();
        for watch in config["watch"].as_array().ok_or("watch must be an array")? {
            let position = pos(watch)?;
            watches.push(match world.chunk_snapshot(position)? {
                Some(chunk) => snapshot(&chunk),
                None => json!({"pos": [position.x, position.z], "absent": true}),
            });
        }
        serde_json::to_writer(
            &mut out,
            &json!({
                "request": index as i64 - bootstrap.len() as i64,
                "phase": if index < bootstrap.len() { "bootstrap" } else { "requests" },
                "spec": spec, "seed": config["seed"],
                "snapshot": snapshot(&result.chunk), "watch": watches,
                "complete": coverage.is_complete(), "stages": stages,
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
            "request {index}: {position:?} {target}, complete={}",
            coverage.is_complete()
        );
    }
    Ok(())
}
