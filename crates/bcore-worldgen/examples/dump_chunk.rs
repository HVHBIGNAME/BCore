//! Dump one world's generated chunks and coverage as newline-delimited JSON.
use bcore_core::ChunkPos;
use bcore_worldgen::generation::ChunkStatus;
use bcore_worldgen::{GenerationCoverage, GenerationWorld};
use serde_json::{json, Value};
use std::env;
use std::io::{self, BufWriter, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 || args.len() % 2 != 0 {
        eprintln!("usage: dump_chunk <seed> <chunk_x> <chunk_z> [<chunk_x> <chunk_z> ...]");
        std::process::exit(2);
    }
    let seed: i64 = args[1].parse().expect("seed must be an i64");
    let world = GenerationWorld::new(seed);
    let mut output = BufWriter::new(io::stdout().lock());
    for pair in args[2..].chunks_exact(2) {
        let x: i32 = pair[0].parse().expect("chunk x must be an i32");
        let z: i32 = pair[1].parse().expect("chunk z must be an i32");
        dump(&world, x, z, &mut output)?;
    }
    output.flush()?;
    Ok(())
}

fn dump(
    world: &GenerationWorld,
    x: i32,
    z: i32,
    output: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = world.generate_chunk(ChunkPos::new(x, z))?;
    let chunk = &result.chunk;
    let heights: Vec<_> = (0..16)
        .flat_map(|z| (0..16).map(move |x| chunk.height_at(x, z)))
        .collect();
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
    let mut block_entities: Vec<_> = chunk
        .block_entities()
        .iter()
        .map(|(&(lx, y, lz), data)| {
            let pos = (x * 16 + lx as i32, y, z * 16 + lz as i32);
            json!({"pos":pos,"type":data.type_id(),"nbt":data.full_data(pos)})
        })
        .collect();
    block_entities.extend(chunk.feature_block_entities().iter().map(
        |(&(lx, y, lz), data)| {
            let pos = (x * 16 + lx as i32, y, z * 16 + lz as i32);
            json!({"pos":pos,"type":data.type_id,"nbt":data.full_data,"typed_nbt":data.typed_data})
        },
    ));
    block_entities.extend(
        chunk
            .pending_block_entities()
            .iter()
            .map(|(&(lx, y, lz), data)| {
                let pos = (x * 16 + lx as i32, y, z * 16 + lz as i32);
                let nbt = data.full_nbt().expect("validated pending proto NBT");
                json!({"pos":pos,"pending":true,"nbt":nbt.to_json(),"typed_nbt":data.typed_data})
            }),
    );
    let generated: Vec<_> = chunk
        .entities()
        .iter()
        .map(|e| json!({"pos": e.position(), "type": e.type_id(), "nbt": e.data()}))
        .collect();
    serde_json::to_writer(
        &mut *output,
        &json!({
            "seed": world.seed(), "x": x, "z": z,
            "states": chunk.states(), "heights": heights, "biomes": biomes,
            "worldgen_heightmaps": chunk.worldgen_heightmaps(),
            "light": chunk.light(),
            "block_entities": block_entities, "entities": generated,
            "structure_entity_requests": chunk.structure_entities(),
            "structures": chunk.structures().native_data(chunk.pos),
            "tick_requests": chunk.tick_requests(),
            "postprocessing": chunk.postprocessing_positions(),
            "generation_coverage": coverage_json(&result.coverage),
        }),
    )?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn coverage_json(coverage: &GenerationCoverage) -> Value {
    let stages: Vec<_> = ChunkStatus::ALL
        .into_iter()
        .map(|status| {
            let progress = coverage.target.stage(status);
            json!({
                "status": status.name(), "state": format!("{:?}", progress.state),
                "attempts": progress.attempts, "missing": progress.missing,
            })
        })
        .collect();
    let layers: Vec<_> = coverage
        .layers
        .iter()
        .map(|layer| {
            json!({
                "status": layer.status.name(), "radius": layer.radius,
                "required_chunks": layer.required_chunks, "complete": layer.complete,
                "partial": layer.partial, "pending": layer.pending, "failed": layer.failed,
            })
        })
        .collect();
    let sources: Vec<_> = coverage
        .feature_sources
        .iter()
        .map(|source| {
            let completed: Vec<_> = source
                .completed
                .iter()
                .map(|work| {
                    json!({
                        "step": work.step, "index": work.index,
                        "feature": work.feature, "placed": work.placed,
                    })
                })
                .collect();
            let missing: Vec<_> = source
                .missing
                .iter()
                .map(|work| {
                    json!({
                        "step": work.step, "index": work.index,
                        "feature": work.feature, "reason": work.reason,
                    })
                })
                .collect();
            json!({
                "source": [source.source.x, source.source.z], "sequence": source.sequence,
                "mineshafts_processed": source.mineshafts_processed,
                "completed": completed, "missing": missing,
            })
        })
        .collect();
    let missing_stages: Vec<_> = coverage
        .missing_stages
        .iter()
        .map(|stage| {
            json!({
                "status": stage.status.name(), "reason": stage.reason,
            })
        })
        .collect();
    json!({
        "requested_status": coverage.requested_status.name(),
        "complete": coverage.is_complete(),
        "completed_status": coverage.target.completed_status().map(ChunkStatus::name),
        "incoming_sources_finished": coverage.incoming_sources_finished,
        "stages": stages, "layers": layers,
        "feature_sources": sources, "missing_stages": missing_stages,
    })
}
