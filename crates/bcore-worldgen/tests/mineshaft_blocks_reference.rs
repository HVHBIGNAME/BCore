use std::collections::BTreeMap;

use bcore_worldgen::{
    block,
    block_entity::BlockEntity,
    dungeon::DungeonWorld,
    generated_entity::GeneratedEntity,
    ore::OreWorld,
    simplex::WorldgenRandom,
    structure::mineshaft::{
        blocks::{blocks_motion, MineshaftWorld},
        Bounds, Direction, MineType, Piece, PieceKind,
    },
};
use serde_json::{json, Value};

fn position(data: &Value) -> [i32; 3] {
    std::array::from_fn(|i| data[i].as_i64().unwrap() as i32)
}

fn bounds(data: &Value) -> Bounds {
    Bounds {
        min: position(data),
        max: std::array::from_fn(|i| data[i + 3].as_i64().unwrap() as i32),
    }
}

fn direction(id: i64) -> Direction {
    match id {
        0 => Direction::South,
        1 => Direction::West,
        2 => Direction::North,
        3 => Direction::East,
        _ => panic!("invalid direction {id}"),
    }
}

fn piece(tag: &Value) -> Piece {
    let orientation = tag["O"].as_i64().unwrap();
    Piece {
        bounds: bounds(&tag["BB"]),
        depth: tag["GD"].as_i64().unwrap() as i32,
        orientation: (orientation >= 0).then(|| direction(orientation)),
        kind: match tag["id"].as_str().unwrap() {
            "minecraft:msroom" => PieceKind::Room {
                entrances: tag["Entrances"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(bounds)
                    .collect(),
            },
            "minecraft:msstairs" => PieceKind::Stairs,
            "minecraft:mscrossing" => PieceKind::Crossing {
                two_floors: tag["tf"] == 1,
                direction: direction(tag["D"].as_i64().unwrap()),
            },
            "minecraft:mscorridor" => PieceKind::Corridor {
                rails: tag["hr"] == 1,
                spider: tag["sc"] == 1,
                placed_spider: tag["hps"] == 1,
                sections: tag["Num"].as_i64().unwrap() as i32,
            },
            id => panic!("unexpected native piece {id}"),
        },
    }
}

struct World<'a> {
    terrain: &'a str,
    shift: [i32; 3],
    writes: BTreeMap<[i32; 3], u32>,
    block_entities: BTreeMap<[i32; 3], BlockEntity>,
    entities: Vec<GeneratedEntity>,
    postprocessing: Vec<[i32; 3]>,
}

impl World<'_> {
    fn initial(&self, [x, y, z]: [i32; 3]) -> u32 {
        if !(-64..320).contains(&y) {
            return block::AIR;
        }
        let [x, y, z] = [x - self.shift[0], y - self.shift[1], z - self.shift[2]];
        if y >= if self.terrain == "structure_solid" {
            320
        } else {
            96
        } {
            return block::AIR;
        }
        match self.terrain {
            "liquid" => block::WATER,
            "sky" => {
                if y < 32 {
                    block::STONE
                } else {
                    block::AIR
                }
            }
            "pillars" => {
                if y <= 20 || y >= 50 {
                    block::STONE
                } else {
                    block::AIR
                }
            }
            "hanging" => {
                if y >= 42 {
                    block::STONE
                } else {
                    block::AIR
                }
            }
            "gravity" => {
                if y >= 42 {
                    block::SAND
                } else {
                    block::AIR
                }
            }
            "long_down" => {
                if y <= 10 || y == 95 {
                    block::STONE
                } else {
                    block::AIR
                }
            }
            "too_low" => {
                if y <= 9 || y == 95 {
                    block::STONE
                } else {
                    block::AIR
                }
            }
            "long_up" => {
                if y >= 82 {
                    block::STONE
                } else {
                    block::AIR
                }
            }
            "too_high" => {
                if y >= 83 {
                    block::STONE
                } else {
                    block::AIR
                }
            }
            "lava_below" => {
                if y == 20 {
                    block::LAVA
                } else if y >= 42 {
                    block::STONE
                } else {
                    block::AIR
                }
            }
            "inside_water" if [x, y, z] == [5, 33, 7] => block::WATER,
            "protected" if x == 5 && y == 33 => {
                [15, 137, 6996, 8249, 21, 155, 13963, 1][z.rem_euclid(8) as usize]
            }
            _ => block::STONE,
        }
    }
}

impl OreWorld for World<'_> {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        (-64..320)
            .rev()
            .find(|&y| blocks_motion(self.get_block((x, y, z)).unwrap()))
            .map_or(-64, |y| y + 1)
    }

    fn get_block(&self, (x, y, z): (i32, i32, i32)) -> Option<u32> {
        let pos = [x, y, z];
        Some(
            self.writes
                .get(&pos)
                .copied()
                .unwrap_or_else(|| self.initial(pos)),
        )
    }

    fn set_block(&mut self, (x, y, z): (i32, i32, i32), state: u32) -> bool {
        if !(-64..320).contains(&y) {
            return false;
        }
        let pos = [x, y, z];
        if state == self.initial(pos) {
            self.writes.remove(&pos);
        } else {
            self.writes.insert(pos, state);
        }
        if self
            .block_entities
            .get(&pos)
            .is_some_and(|be| !be.matches_state(state))
        {
            self.block_entities.remove(&pos);
        }
        true
    }
}

impl DungeonWorld for World<'_> {
    fn set_block_entity(&mut self, (x, y, z): (i32, i32, i32), data: BlockEntity) {
        assert!(data.matches_state(self.get_block((x, y, z)).unwrap()));
        self.block_entities.insert([x, y, z], data);
    }
}

impl MineshaftWorld for World<'_> {
    fn mineshaft_blocked_biome(&self, _: [i32; 3]) -> bool {
        self.terrain == "blocked"
    }
    fn add_entity(&mut self, entity: GeneratedEntity) {
        self.entities.push(entity);
    }
    fn mark_for_postprocessing(&mut self, pos: [i32; 3]) {
        self.postprocessing.push(pos);
    }
}

#[test]
fn mineshaft_blocks_rng_entities_and_clipped_piece_state_match_native_26_1() {
    let fixture: Value =
        serde_json::from_str(include_str!("../data/mineshaft_blocks_26_1.json")).unwrap();
    let mut minecarts = 0;
    let mut spawners = 0;
    for (index, sample) in fixture["samples"].as_array().unwrap().iter().enumerate() {
        let mut piece = piece(&sample["piece"]);
        let mine_type = if sample["piece"]["MST"] == 0 {
            MineType::Normal
        } else {
            MineType::Mesa
        };
        let seed = sample["seed"].as_i64().unwrap();
        let terrain = sample["terrain"].as_str().unwrap();
        let label = format!(
            "sample {index} seed {seed} {terrain} {:?} {:?}",
            piece.bounds, piece.orientation
        );
        let mut random = WorldgenRandom::new(seed);
        let mut world = World {
            terrain,
            shift: position(&sample["shift"]),
            writes: BTreeMap::new(),
            block_entities: BTreeMap::new(),
            entities: Vec::new(),
            postprocessing: Vec::new(),
        };
        assert_eq!(piece.save_data(mine_type), sample["piece"], "{label}");
        for (pass, clip) in sample["clips"].as_array().unwrap().iter().enumerate() {
            piece.post_process(mine_type, &mut world, &mut random, bounds(clip));
            assert_eq!(
                piece.save_data(mine_type),
                sample["piece_states"][pass],
                "{label} pass {pass}"
            );
        }
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{label} RNG"
        );
        let mut digest = md5::Context::new();
        for (pos, state) in &world.writes {
            for value in pos {
                digest.consume(value.to_le_bytes());
            }
            digest.consume(state.to_le_bytes());
        }
        assert_eq!(
            world.writes.len() as u64,
            sample["changed_blocks"].as_u64().unwrap(),
            "{label}"
        );
        assert_eq!(
            format!("{:x}", digest.compute()),
            sample["writes_md5"].as_str().unwrap(),
            "{label}"
        );
        let block_entities: Vec<_> = world
            .block_entities
            .iter()
            .map(|(&[x, y, z], be)| be.full_data((x, y, z)))
            .collect();
        assert_eq!(
            json!(block_entities),
            sample["block_entities"],
            "{label} block entities"
        );
        let expected = sample["entities"].as_array().unwrap();
        assert_eq!(world.entities.len(), expected.len(), "{label} minecarts");
        for (entity, expected) in world.entities.iter().zip(expected) {
            assert_eq!(entity.data(), expected["nbt"], "{label} entity NBT");
            assert_eq!(
                json!(entity.position()),
                expected["pos"],
                "{label} entity position"
            );
            assert_eq!(
                entity.type_id(),
                expected["type"].as_u64().unwrap() as u32,
                "{label} entity type"
            );
        }
        world.postprocessing.sort();
        assert_eq!(
            json!(world.postprocessing),
            sample["postprocessing"],
            "{label} shape update marks"
        );
        minecarts += world.entities.len();
        spawners += world.block_entities.len();
    }
    assert!(
        minecarts >= 9 && spawners >= 20,
        "native entity branches must stay covered"
    );
}

fn assert_region_chunk(world: &World<'_>, record: &Value) {
    let (cx, cz) = (
        record["chunk"][0].as_i64().unwrap() as i32,
        record["chunk"][1].as_i64().unwrap() as i32,
    );
    let in_chunk = |p: &&[i32; 3]| p[0] >> 4 == cx && p[2] >> 4 == cz;
    let mut digest = md5::Context::new();
    let mut count = 0;
    for (pos, state) in world.writes.iter().filter(|(p, _)| in_chunk(p)) {
        for v in pos {
            digest.consume(v.to_le_bytes());
        }
        digest.consume(state.to_le_bytes());
        count += 1;
    }
    assert_eq!(
        count,
        record["changed_blocks"].as_u64().unwrap(),
        "chunk ({cx},{cz}) count"
    );
    assert_eq!(
        format!("{:x}", digest.compute()),
        record["writes_md5"].as_str().unwrap(),
        "chunk ({cx},{cz}) blocks"
    );
    let bes: Vec<_> = world
        .block_entities
        .iter()
        .filter(|(p, _)| in_chunk(p))
        .map(|(&[x, y, z], be)| be.full_data((x, y, z)))
        .collect();
    assert_eq!(
        json!(bes),
        record["block_entities"],
        "chunk ({cx},{cz}) spawners"
    );
    let carts: Vec<_> = world
        .entities
        .iter()
        .filter(|e| in_chunk(&&e.block_pos()))
        .map(|e| match e {
            GeneratedEntity::ChestMinecart { loot_seed, .. } => {
                json!({"pos": e.position(),"loot_seed": loot_seed})
            }
        })
        .collect();
    assert_eq!(
        json!(carts),
        record["minecarts"],
        "chunk ({cx},{cz}) minecarts"
    );
}

#[test]
fn overlapping_starts_and_independent_chunk_requests_match_native_region() {
    use bcore_core::ChunkPos;
    use bcore_worldgen::{
        structure::mineshaft::{region::RegionPlan, MineshaftLayout},
        WorldGenerator,
    };
    let fixture: Value =
        serde_json::from_str(include_str!("../data/mineshaft_regions_26_1.json")).unwrap();
    for sample in fixture["samples"].as_array().unwrap() {
        let seed = sample["seed"].as_i64().unwrap();
        let starts: BTreeMap<_, _> = sample["starts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|start| {
                let key = (
                    start["source"][0].as_i64().unwrap() as i32,
                    start["source"][1].as_i64().unwrap() as i32,
                );
                let layout = MineshaftLayout {
                    mine_type: if start["id"] == "minecraft:mineshaft" {
                        MineType::Normal
                    } else {
                        MineType::Mesa
                    },
                    pieces: start["pieces"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(piece)
                        .collect(),
                };
                // Also exercise production start generation, not only deserialization.
                let biome = if layout.mine_type == MineType::Normal {
                    "plains"
                } else {
                    "badlands"
                };
                let generator = WorldGenerator::new(seed);
                let generated = MineshaftLayout::for_chunk(
                    seed,
                    ChunkPos::new(key.0, key.1),
                    |_| bcore_worldgen::biome::id(biome).unwrap(),
                    |x, z| generator.base_height_vanilla(x, z),
                )
                .unwrap();
                assert_eq!(generated, layout, "start {key:?} seed {seed}");
                (key, layout)
            })
            .collect();
        let empty_world = || World {
            terrain: "structure_solid",
            shift: [0, 0, 0],
            writes: BTreeMap::new(),
            block_entities: BTreeMap::new(),
            entities: Vec::new(),
            postprocessing: Vec::new(),
        };
        let records = sample["chunks"].as_array().unwrap();
        let pos = |r: &Value| {
            ChunkPos::new(
                r["chunk"][0].as_i64().unwrap() as i32,
                r["chunk"][1].as_i64().unwrap() as i32,
            )
        };
        let start_at = |p: ChunkPos| starts.get(&(p.x, p.z)).cloned();
        let mut complete = RegionPlan::new(records.iter().map(pos), start_at);
        let mut whole = empty_world();
        complete.place(seed, &mut whole);
        for record in records {
            assert_region_chunk(&whole, record);
            assert_eq!(
                complete
                    .structure_data(pos(record))
                    .native_data(pos(record))["references"],
                record["references"]
            );
        }
        // Reverse request order, rebuilding each chunk independently. Earlier
        // spider clips must be replayed, without replaying the entire region.
        for record in records.iter().rev() {
            let mut plan = RegionPlan::new([pos(record)], start_at);
            let mut independent = empty_world();
            plan.place(seed, &mut independent);
            assert_region_chunk(&independent, record);
        }
    }
}

#[test]
fn fence_connections_and_wall_torch_survival_match_native_shape_updates() {
    let fixture: Value =
        serde_json::from_str(include_str!("../data/mineshaft_blocks_26_1.json")).unwrap();
    let deltas = [
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
        [-1, 0, 0],
        [1, 0, 0],
    ];
    for row in fixture["shape_checks"].as_array().unwrap() {
        let mut world = World {
            terrain: "sky",
            shift: [0, 0, 0],
            writes: BTreeMap::new(),
            block_entities: BTreeMap::new(),
            entities: Vec::new(),
            postprocessing: Vec::new(),
        };
        world.set_block((0, 32, 0), row["state"].as_u64().unwrap() as u32);
        for (i, [x, y, z]) in deltas.iter().enumerate() {
            world.set_block(
                (*x, 32 + y, *z),
                row["neighbors"][i].as_u64().unwrap() as u32,
            );
        }
        let state = bcore_worldgen::structure::mineshaft::blocks::updated_shape(&world, [0, 32, 0]);
        assert_eq!(state, row["updated"].as_u64().unwrap() as u32, "{row}");
    }
}
