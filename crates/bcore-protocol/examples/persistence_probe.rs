//! Edit one isolated saved chunk through the production persistence codec.
use std::error::Error;
use std::path::PathBuf;

use bcore_protocol::chunk_store::{ChunkStore, FORMAT_VERSION};
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err("usage: persistence_probe <world> <seed> <chunk-x> <chunk-z>".into());
    }
    let root = PathBuf::from(&args[0]).canonicalize()?;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .ok_or("workspace root unavailable")?
        .join("target")
        .canonicalize()?;
    if !root.starts_with(&workspace)
        || root.file_name().and_then(|name| name.to_str()) != Some("world")
    {
        return Err("persistence probe requires an isolated target/ world".into());
    }
    let seed: i64 = args[1].parse()?;
    let cx: i32 = args[2].parse()?;
    let cz: i32 = args[3].parse()?;
    let x = cx
        .checked_mul(16)
        .and_then(|x| x.checked_add(7))
        .ok_or("block X overflow")?;
    let z = cz
        .checked_mul(16)
        .and_then(|z| z.checked_add(7))
        .ok_or("block Z overflow")?;
    let store = ChunkStore::at(&root);
    store.prepare_world(seed)?;
    let mut column = store.load(cx, cz)?.ok_or("probe chunk was not persisted")?;
    let before_state = column.get(7, 0, 7).ok_or("probe position unavailable")?;
    let state = bcore_registry::vanilla()
        .states
        .iter()
        .find(|state| state.name == "minecraft:diamond_block" && state.default)
        .ok_or("diamond block default unavailable")?
        .id;
    if before_state == state {
        return Err("probe position already contains the marker block".into());
    }
    let path = store.chunk_path(cx, cz);
    let before = std::fs::read(&path)?;
    if !column.set(7, 0, 7, state) {
        return Err("probe block edit rejected".into());
    }
    store.save(cx, cz, &column)?;
    let after = std::fs::read(&path)?;
    let loaded = store
        .load(cx, cz)?
        .ok_or("edited probe chunk disappeared")?;
    if loaded.get(7, 0, 7) != Some(state) || before == after {
        return Err("edited chunk did not round-trip through the production codec".into());
    }
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": "bcore.persistence-mutation.v1", "format_version": FORMAT_VERSION,
            "minecraft": bcore_registry::MC_VERSION, "protocol": bcore_registry::PROTOCOL_VERSION,
            "data_version": bcore_registry::DATA_VERSION, "seed": seed.to_string(),
            "chunk": [cx, cz], "block": [x, 0, z], "local": [7, 0, 7],
            "before_state": before_state, "state": state, "block_name": "minecraft:diamond_block",
            "path": path, "before_sha256": format!("{:x}", Sha256::digest(&before)),
            "sha256": format!("{:x}", Sha256::digest(&after))
        }))?
    );
    Ok(())
}
