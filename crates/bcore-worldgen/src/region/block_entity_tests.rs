//! Differential lifecycle tests against original chunks/regions, not an eager
//! fixture-world replacement for WorldGenRegion.getBlockEntity.
use super::*;
use crate::block_entity::PendingBlockEntity;
use crate::feature_world::Pos;
use crate::simplex::JavaRandom;
use crate::structure::template::{
    BlockInfo, Nbt, PlacementSettings, ProcessorRandom, StructureTemplate,
};
use crate::structure::template_pool::StructureAssets;
use serde_json::Value;

fn fixture() -> Value {
    let data: Value = serde_json::from_str(include_str!(
        "../../data/deferred_block_entities_26_1_v1.json"
    ))
    .unwrap();
    assert_eq!(
        data["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(data["schema"], 1);
    assert_eq!(data["cases"].as_array().unwrap().len(), 39);
    data
}

fn point(v: &Value) -> Pos {
    let p: [i32; 3] = serde_json::from_value(v.clone()).unwrap();
    (p[0], p[1], p[2])
}

fn nbt(v: &Value) -> Nbt {
    serde_json::from_value(v["nbt"].clone()).unwrap()
}

fn observation<'a>(row: &'a Value, boundary: &str) -> &'a Value {
    row["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["boundary"] == boundary)
        .unwrap()
}

fn source(row: &Value) -> ChunkPos {
    ChunkPos::new(
        row["source"][0].as_i64().unwrap() as i32,
        row["source"][1].as_i64().unwrap() as i32,
    )
}

fn fresh(source: ChunkPos) -> FeatureRegion {
    let mut region = FeatureRegion::shared(WorldGenerator::new(846692123413862008));
    region.begin_source(
        source,
        ChunkStatus::Features,
        crate::generation::graph::layer_positions(source, 8)
            .map(|p| ((p.x, p.z), ChunkStatus::Carvers))
            .collect(),
    );
    region
}

fn check(region: &FeatureRegion, expected: &Value, label: &str) {
    let p @ (x, y, z) = point(&expected["pos"]);
    let chunk = region.chunk(x >> 4, z >> 4);
    let local = ((x & 15) as usize, y, (z & 15) as usize);
    assert_eq!(
        chunk.get(local.0, y, local.2),
        Some(expected["state"].as_u64().unwrap() as u32),
        "{label}"
    );
    assert_eq!(
        chunk.pending_block_entities().len(),
        expected["pending_count"].as_u64().unwrap() as usize,
        "{label} pending"
    );
    assert_eq!(
        chunk.block_entities().len() + chunk.feature_block_entities().len(),
        expected["loaded_count"].as_u64().unwrap() as usize,
        "{label} loaded"
    );
    if expected["pending"].is_null() {
        assert!(
            !chunk.pending_block_entities().contains_key(&local),
            "{label}"
        );
    } else {
        assert_eq!(
            chunk.pending_block_entities()[&local].full_nbt().unwrap(),
            nbt(&expected["pending"]),
            "{label} pending payload"
        );
    }
    if let Some(entity) = chunk.feature_block_entities().get(&local) {
        assert!(
            entity.valid_for(chunk.get(local.0, y, local.2).unwrap(), p),
            "{label}"
        );
        assert_eq!(
            entity.full_nbt().unwrap(),
            nbt(&expected["full"]),
            "{label} typed full"
        );
        assert_eq!(
            entity.update_nbt().unwrap(),
            nbt(&expected["update"]),
            "{label} typed update"
        );
    } else if let Some(entity) = chunk.block_entities().get(&local) {
        // The existing core hive representation stores each age as i32 in BCC;
        // protocol tests separately assert those bytes and the empty update tag.
        assert_eq!(
            entity.full_data(p),
            nbt(&expected["full"]).to_json(),
            "{label} core full"
        );
        assert_eq!(
            entity.update_data(),
            nbt(&expected["update"]).to_json(),
            "{label} core update"
        );
    } else {
        assert!(
            expected["full"].is_null() && expected["update"].is_null(),
            "{label}"
        );
    }
}

#[test]
fn deferred_native_proto_writes_save_without_lookup_and_materialize_only_on_lookup() {
    let data = fixture();
    let mut checked = 0;
    for row in data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "basic")
    {
        let label = row["name"].as_str().unwrap();
        let mut region = fresh(source(row));
        let initial = observation(row, "write/no lookup");
        let p = point(&initial["pos"]);
        let state = initial["state"].as_u64().unwrap() as u32;
        assert!(row["written"].as_bool().unwrap() && row["proto_lookup_null"].as_bool().unwrap());
        assert!(region.set_feature_block(p, state, row["flags"].as_i64().unwrap() as i32));
        check(&region, initial, label);
        let saved = observation(row, "proto save/no lookup");
        assert_eq!(
            nbt(&saved["saved"]),
            nbt(&initial["pending"]),
            "{label} native save is inert"
        );
        check(&region, &saved["after"], label);
        assert!(region.materialize_block_entity_at(p).unwrap());
        check(&region, observation(row, "region lookup"), label);
        assert!(region.materialize_block_entity_at(p).unwrap());
        assert!(
            row["lookup_idempotent"].as_bool().unwrap()
                && row["loaded_rewrite_retained"].as_bool().unwrap()
        );
        assert!(region.set_feature_block(p, state, row["flags"].as_i64().unwrap() as i32));
        check(
            &region,
            &observation(row, "same-state rewrite/loaded save")["after"],
            label,
        );
        assert!(region.set_feature_block(p, block::AIR, 18));
        check(&region, observation(row, "remove to air"), label);
        checked += 1;
    }
    assert_eq!(checked, 16);
}

#[test]
fn deferred_native_saved_tags_remain_typed_and_pending_until_region_lookup() {
    let data = fixture();
    let mut checked = 0;
    for row in data["cases"].as_array().unwrap() {
        for saved in row["observations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v.get("restored").is_some())
        {
            let before = &saved["restored"];
            let p @ (x, y, z) = point(&before["pos"]);
            let mut region = fresh(ChunkPos::new(x >> 4, z >> 4));
            let state = before["state"].as_u64().unwrap() as u32;
            let local = ((x & 15) as usize, y, (z & 15) as usize);
            // Native SerializableChunkData.read restored these tags as pending,
            // including those saved from an already materialized proto entity.
            assert_eq!(before["loaded_count"], 0);
            assert!(region.set_feature_block(p, state, 18));
            assert!(region.chunk_mut(x >> 4, z >> 4).set_pending_block_entity(
                local.0,
                y,
                local.2,
                PendingBlockEntity::from_nbt(&nbt(&before["pending"]))
            ));
            check(&region, before, row["name"].as_str().unwrap());
            assert!(region.materialize_block_entity_at(p).unwrap());
            check(
                &region,
                &saved["restored_lookup"],
                row["name"].as_str().unwrap(),
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 55);
    for row in data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "saved")
    {
        let before = observation(row, "installed saved pending NBT");
        let p @ (x, y, z) = point(&before["pos"]);
        let mut region = fresh(source(row));
        let state = before["state"].as_u64().unwrap() as u32;
        let pending = nbt(&before["pending"]);
        assert_eq!(pending.get("keepPacked"), Some(&Nbt::Byte(1)));
        assert!(region.set_feature_block(p, state, 18));
        assert!(region.chunk_mut(x >> 4, z >> 4).set_pending_block_entity(
            (x & 15) as usize,
            y,
            (z & 15) as usize,
            PendingBlockEntity::from_nbt(&pending)
        ));
        check(
            &region,
            &observation(row, "saved pending proto save")["after"],
            "raw saved pending",
        );
        assert!(region.materialize_block_entity_at(p).unwrap());
        check(
            &region,
            &observation(row, "saved pending region lookup")["after"],
            "loaded saved pending",
        );
    }
}

#[test]
fn deferred_native_template_absent_nbt_differs_from_empty_or_saved_nbt() {
    let data = fixture();
    let blocks = &StructureAssets::bundled().blocks;
    let mut checked = 0;
    for row in data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "template")
    {
        let expected = observation(row, "after native template placement");
        let p = point(&expected["pos"]);
        let state = expected["state"].as_u64().unwrap() as u32;
        let input = nbt(&row["template_input"]);
        let load = input.get("blocks").unwrap().list().unwrap()[0]
            .get("nbt")
            .cloned();
        let template = StructureTemplate::new(
            (1, 1, 1),
            vec![vec![BlockInfo {
                pos: (0, 0, 0),
                state,
                nbt: load,
            }]],
            Vec::new(),
            blocks,
        )
        .unwrap();
        let mut region = fresh(source(row));
        let result = template
            .place_with_effects(
                &mut region,
                &mut JavaRandom::new(12345),
                &mut ProcessorRandom(None),
                blocks,
                p,
                p,
                &PlacementSettings::default(),
                &mut |world, effect| world.apply_template_effect(source(row), effect),
            )
            .unwrap();
        assert_eq!(result.placed, row["placed"].as_bool().unwrap());
        check(&region, expected, row["name"].as_str().unwrap());
        check(
            &region,
            &observation(row, "template proto save")["after"],
            row["name"].as_str().unwrap(),
        );
        checked += 1;
    }
    assert_eq!(checked, 15);
}

#[test]
fn deferred_lookup_boundary_matches_post_conversion_native_data_without_retiring_other_work() {
    let data = fixture();
    let mut checked = 0;
    for row in data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "basic")
    {
        let before = observation(row, "after conversion/no lookup");
        let p @ (x, y, z) = point(&before["pos"]);
        let mut region = fresh(ChunkPos::new(x >> 4, z >> 4));
        assert!(region.set_feature_block(p, before["state"].as_u64().unwrap() as u32, 18));
        check(&region, before, "conversion retains pending");
        let chunk = region.chunk_mut(x >> 4, z >> 4);
        chunk.capture_worldgen_heightmaps(true);
        chunk.postprocessing.extend([(15, y, 1), (15, y, 1)]);
        let request = TickRequest {
            block_pos: [x, y, z],
            target: TickTarget::Fluid(2),
            delay: -5,
        };
        assert!(chunk.add_tick_request(request));
        assert!(chunk.add_tick_request(request));
        let old = chunk.clone();
        assert_eq!(chunk.materialize_block_entities().unwrap(), 1);
        assert_eq!(chunk.materialize_block_entities().unwrap(), 0);
        assert_eq!(chunk.states(), old.states());
        assert_eq!(chunk.worldgen_heightmaps(), old.worldgen_heightmaps());
        assert_eq!(
            chunk.postprocessing_positions(),
            old.postprocessing_positions()
        );
        assert_eq!(chunk.tick_requests(), old.tick_requests());
        check(
            &region,
            &observation(row, "level save")["after"],
            "explicit materialization",
        );
        assert_eq!(
            nbt(&observation(row, "level save")["saved"]).get("keepPacked"),
            Some(&Nbt::Byte(0))
        );
        checked += 1;
    }
    assert_eq!(checked, 16);
}

#[test]
fn deferred_hive_source_callback_reaches_the_neighbour_and_retains_occupants() {
    use crate::tree::standing::StandingTreeWorld;
    let data = fixture();
    let row = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "BEE_NEST/source-callback")
        .unwrap();
    let initial = observation(row, "source write/no callback");
    let p = point(&initial["pos"]);
    let mut region = fresh(source(row));
    assert!(region.set_feature_block(p, initial["state"].as_u64().unwrap() as u32, 2));
    check(&region, initial, "hive pending");
    for ticks in [0, 599, -7] {
        region.store_bee(p, ticks);
    }
    region.transfer_tree_effects().unwrap();
    check(
        &region,
        &observation(row, "source getBlockEntity/mutation")["after"],
        "stored bees",
    );
    assert!(region.set_feature_block(p, initial["state"].as_u64().unwrap() as u32, 18));
    check(
        &region,
        &observation(row, "rewrite after callback")["after"],
        "retained bees",
    );
    assert!(
        region.owned_chunk(source(row)).is_none(),
        "callback must not attach the hive to its source"
    );
}

#[test]
fn deferred_hive_callback_appends_to_an_already_loaded_saved_hive() {
    use crate::structure::template::{TemplateBlockEntity, TemplateEffect};
    use crate::tree::standing::StandingTreeWorld;
    let data = fixture();
    let row = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "BEE_NEST/source-callback")
        .unwrap();
    let initial = observation(row, "source write/no callback");
    let expected = &observation(row, "source getBlockEntity/mutation")["after"];
    let p = point(&initial["pos"]);
    let state = initial["state"].as_u64().unwrap() as u32;
    let full = nbt(&expected["full"]);
    let occupants = full.get("bees").and_then(Nbt::list).unwrap();
    assert_eq!(occupants.len(), 3);
    let owner = ChunkPos::new(p.0 >> 4, p.2 >> 4);
    let local = ((p.0 & 15) as usize, p.1, (p.2 & 15) as usize);
    for saved_pending in [false, true] {
        for prefix_len in 0..occupants.len() {
            let mut prefix = full.clone();
            prefix.compound_mut().unwrap().insert(
                "bees".into(),
                Nbt::List {
                    element_type: if prefix_len == 0 { 0 } else { 10 },
                    values: occupants[..prefix_len].to_vec(),
                },
            );
            let mut region = fresh(source(row));
            assert!(region.set_feature_block(p, state, 2));
            if saved_pending {
                assert!(region.owned_chunk_mut(owner).set_pending_block_entity(
                    local.0,
                    local.1,
                    local.2,
                    PendingBlockEntity::from_nbt(&prefix),
                ));
                assert!(region.materialize_block_entity_at(p).unwrap());
            } else {
                let loaded = TemplateBlockEntity::from_load(
                    &StructureAssets::bundled().blocks,
                    state,
                    p,
                    prefix.clone(),
                )
                .unwrap();
                region
                    .apply_template_effect(source(row), TemplateEffect::BlockEntity(loaded))
                    .unwrap();
            }
            let prior = region.owned_chunk(owner).unwrap();
            let mut ticks: Vec<_> = occupants[..prefix_len]
                .iter()
                .map(|bee| bee.get("ticks_in_hive").and_then(Nbt::int).unwrap())
                .collect();
            assert_eq!(region.beehive_snapshots()[&p], ticks);
            for bee in &occupants[prefix_len..] {
                let age = bee.get("ticks_in_hive").and_then(Nbt::int).unwrap();
                region.store_bee(p, age);
                ticks.push(age);
                assert_eq!(region.beehive_snapshots()[&p], ticks);
                region.transfer_tree_effects().unwrap();
                assert_eq!(region.beehive_snapshots()[&p], ticks);
                let transferred = region.owned_chunk(owner).unwrap();
                region.transfer_tree_effects().unwrap();
                assert_eq!(*transferred, *region.owned_chunk(owner).unwrap());
                assert!(region.set_feature_block(p, state, 18));
                assert_eq!(*transferred, *region.owned_chunk(owner).unwrap());
            }
            assert_eq!(
                prior.feature_block_entities()[&local].full_nbt().unwrap(),
                prefix
            );
            check(
                &region,
                expected,
                "loaded hive append preserves earlier occupants and types",
            );
            assert!(region.owned_chunk(source(row)).is_none());
        }
    }
}

#[test]
fn deferred_invalid_ownership_missing_tags_and_unsupported_load_are_not_silently_materialized() {
    let owner = ChunkPos::new(0, 0);
    let mut region = fresh(owner);
    let state = StructureAssets::bundled()
        .blocks
        .default_state("chest")
        .unwrap();
    let chunk = region.owned_chunk_mut(owner);
    chunk.set(0, 80, 0, state);
    assert!(
        !chunk.materialize_block_entity(0, 80, 0).unwrap(),
        "plain ProtoChunk setBlockState does not create NBT"
    );
    assert!(!chunk.set_pending_block_entity(0, 80, 0, PendingBlockEntity::dummy((16, 80, 0))));
    assert!(chunk.set_pending_block_entity(0, 80, 0, PendingBlockEntity::dummy((0, 80, 0))));
    chunk.set(1, 80, 0, state);
    let mut unknown = PendingBlockEntity::dummy((1, 80, 0)).full_nbt().unwrap();
    unknown.compound_mut().unwrap().insert(
        "id".into(),
        Nbt::String("minecraft:missing_native_codec".into()),
    );
    assert!(chunk.set_pending_block_entity(1, 80, 0, PendingBlockEntity::from_nbt(&unknown)));
    let before = chunk.clone();
    assert!(chunk.materialize_block_entities().is_err());
    assert_eq!(
        *chunk, before,
        "an unsupported batch must preserve raw evidence"
    );
    assert!(region.materialize_block_entity_at((16 * 9, 80, 0)).is_err());
}
