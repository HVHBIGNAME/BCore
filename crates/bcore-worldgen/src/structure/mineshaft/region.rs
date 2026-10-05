//! Chunk-clipped structure placement with explicit spider-corridor dependencies.
//! The canonical chunk order is X then Z. Only earlier clips of spider pieces
//! carry mutable state into a chunk; the other pieces are horizontally local.
use std::collections::{BTreeMap, BTreeSet};

use super::blocks::MineshaftWorld;
use super::{Bounds, MineType, MineshaftLayout, PieceKind};
use crate::simplex::WorldgenRandom;
use crate::{ChunkPos, MAX_Y, MIN_Y};

type Key = (i32, i32);

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StructureData {
    pub mineshaft_start: Option<MineshaftLayout>,
    pub references: Vec<([i32; 2], MineType)>,
    #[serde(default)]
    pub jigsaw_starts: BTreeMap<String, crate::structure::jigsaw::JigsawStart>,
    /// Per-structure native LongOpenHashSet iteration order.
    #[serde(default)]
    pub jigsaw_references: BTreeMap<String, Vec<[i32; 2]>>,
    #[serde(default)]
    pub scattered_starts: BTreeMap<String, crate::structure::scattered::ScatteredStart>,
    #[serde(default)]
    pub scattered_references: BTreeMap<String, Vec<[i32; 2]>>,
}

impl StructureData {
    pub fn is_empty(&self) -> bool {
        self.mineshaft_start.is_none()
            && self.references.is_empty()
            && self.jigsaw_starts.is_empty()
            && self.jigsaw_references.is_empty()
            && self.scattered_starts.is_empty()
            && self.scattered_references.is_empty()
    }

    pub fn valid_for(&self, owner: ChunkPos) -> bool {
        use crate::structure::scattered::ScatteredKind;
        if self.scattered_starts.len() > ScatteredKind::ALL.len()
            || self.scattered_references.len() > ScatteredKind::ALL.len()
            || self
                .scattered_starts
                .iter()
                .any(|(name, start)| !start.valid_for(name, owner))
            || self.scattered_references.iter().any(|(name, sources)| {
                let mut unique = BTreeSet::new();
                !ScatteredKind::ALL.iter().any(|kind| kind.name() == name)
                    || sources.len() > 289
                    || sources.iter().any(|p| {
                        (i64::from(p[0]) - i64::from(owner.x)).abs() > 8
                            || (i64::from(p[1]) - i64::from(owner.z)).abs() > 8
                            || !unique.insert(*p)
                    })
            })
        {
            return false;
        }
        let assets = crate::structure::template_pool::StructureAssets::bundled();
        if self.jigsaw_starts.len() > assets.structure_metadata.len()
            || self.jigsaw_references.len() > assets.structure_metadata.len()
            || self
                .jigsaw_starts
                .iter()
                .any(|(name, start)| !start.valid_for(name, owner))
            || self.jigsaw_references.iter().any(|(name, references)| {
                let mut unique = BTreeSet::new();
                !assets.structure_metadata.contains_key(name)
                    || references.len() > 289
                    || references.iter().any(|p| {
                        (i64::from(p[0]) - i64::from(owner.x)).abs() > 8
                            || (i64::from(p[1]) - i64::from(owner.z)).abs() > 8
                            || !unique.insert(*p)
                    })
            })
        {
            return false;
        }
        let mut unique = BTreeSet::new();
        if self.references.len() > 289
            || self.references.iter().any(|(p, _)| {
                (i64::from(p[0]) - i64::from(owner.x)).abs() > 8
                    || (i64::from(p[1]) - i64::from(owner.z)).abs() > 8
                    || !unique.insert(*p)
            })
        {
            return false;
        }
        let Some(start) = &self.mineshaft_start else {
            return true;
        };
        if start.pieces.is_empty() || start.pieces.len() > 16384 {
            return false;
        }
        let valid_box = |b: Bounds| {
            (0..3).all(|axis| (0..384).contains(&(i64::from(b.max[axis]) - i64::from(b.min[axis]))))
                && [0, 2].into_iter().all(|axis| {
                    let origin = i64::from(if axis == 0 { owner.x } else { owner.z }) * 16;
                    (i64::from(b.min[axis]) - origin).abs() <= 128
                        && (i64::from(b.max[axis]) - origin).abs() <= 128
                })
                && b.min[1] >= MIN_Y - 384
                && b.max[1] <= MAX_Y + 384
        };
        start.pieces.iter().enumerate().all(|(i, piece)| {
            valid_box(piece.bounds)
                && (0..=9).contains(&piece.depth)
                && match &piece.kind {
                    PieceKind::Room { entrances } => {
                        i == 0
                            && piece.depth == 0
                            && piece.orientation.is_none()
                            && entrances.len() <= 64
                            && entrances.iter().all(|b| valid_box(*b))
                    }
                    PieceKind::Corridor {
                        rails,
                        spider,
                        sections,
                        ..
                    } => {
                        i > 0
                            && piece.orientation.is_some()
                            && !(*rails && *spider)
                            && (1..=4).contains(sections)
                    }
                    PieceKind::Crossing { .. } => i > 0 && piece.orientation.is_none(),
                    PieceKind::Stairs => i > 0 && piece.orientation.is_some(),
                }
        })
    }

    pub fn native_data(&self, pos: ChunkPos) -> serde_json::Value {
        use serde_json::json;
        let mut starts = serde_json::Map::new();
        if let Some(start) = &self.mineshaft_start {
            starts.insert(start.mine_type.name().to_owned(), json!({
                "id": start.mine_type.name(), "ChunkX": pos.x, "ChunkZ": pos.z,
                "references": 0,
                "Children": start.pieces.iter().map(|p| p.save_data(start.mine_type)).collect::<Vec<_>>()
            }));
        }
        for (name, start) in &self.jigsaw_starts {
            starts.insert(name.clone(), start.to_nbt(name, pos).to_json());
        }
        for (name, start) in &self.scattered_starts {
            starts.insert(name.clone(), start.to_nbt().to_json());
        }
        let mut references = serde_json::Map::new();
        for mine_type in [MineType::Normal, MineType::Mesa] {
            let values: Vec<_> = self
                .references
                .iter()
                .filter(|(_, ty)| *ty == mine_type)
                .map(|([x, z], _)| pack((*x, *z)) as i64)
                .collect();
            if !values.is_empty() {
                references.insert(mine_type.name().to_owned(), json!(values));
            }
        }
        for (name, sources) in self
            .jigsaw_references
            .iter()
            .chain(&self.scattered_references)
        {
            if !sources.is_empty() {
                references.insert(
                    name.clone(),
                    json!(sources
                        .iter()
                        .map(|&[x, z]| pack((x, z)) as i64)
                        .collect::<Vec<_>>()),
                );
            }
        }
        json!({"starts": starts, "references": references})
    }
}

pub struct RegionPlan {
    starts: BTreeMap<Key, MineshaftLayout>,
    references: BTreeMap<Key, Vec<(Key, MineType)>>,
}

pub fn writable_area(pos: ChunkPos) -> Bounds {
    Bounds {
        min: [pos.x * 16, MIN_Y + 1, pos.z * 16],
        max: [pos.x * 16 + 15, MAX_Y, pos.z * 16 + 15],
    }
}

impl RegionPlan {
    pub fn new(
        requested: impl IntoIterator<Item = ChunkPos>,
        mut start_at: impl FnMut(ChunkPos) -> Option<MineshaftLayout>,
    ) -> Self {
        let mut result = Self {
            starts: BTreeMap::new(),
            references: BTreeMap::new(),
        };
        let mut queried = BTreeSet::new();
        let mut pending: BTreeSet<Key> = requested.into_iter().map(|p| (p.x, p.z)).collect();
        while let Some(target) = pending.pop_first() {
            if result.references.contains_key(&target) {
                continue;
            }
            let clip = writable_area(ChunkPos::new(target.0, target.1));
            let mut references = Vec::new();
            for x in target.0 - 8..=target.0 + 8 {
                for z in target.1 - 8..=target.1 + 8 {
                    let key = (x, z);
                    if queried.insert(key) {
                        if let Some(start) = start_at(ChunkPos::new(x, z)) {
                            result.starts.insert(key, start);
                        }
                    }
                    let Some(start) = result.starts.get(&key) else {
                        continue;
                    };
                    if !start.bounds().intersects(clip) {
                        continue;
                    }
                    references.push((key, start.mine_type));
                    for piece in &start.pieces {
                        if !matches!(piece.kind, PieceKind::Corridor { spider: true, .. })
                            || !piece.bounds.intersects(clip)
                        {
                            continue;
                        }
                        for cx in piece.bounds.min[0] >> 4..=piece.bounds.max[0] >> 4 {
                            for cz in piece.bounds.min[2] >> 4..=piece.bounds.max[2] >> 4 {
                                let dependency = (cx, cz);
                                if dependency < target
                                    && !result.references.contains_key(&dependency)
                                {
                                    pending.insert(dependency);
                                }
                            }
                        }
                    }
                }
            }
            let mut ordered = Vec::new();
            for mine_type in [MineType::Normal, MineType::Mesa] {
                let positions = reference_order(
                    references
                        .iter()
                        .filter(|(_, ty)| *ty == mine_type)
                        .map(|(p, _)| *p),
                );
                ordered.extend(positions.into_iter().map(|p| (p, mine_type)));
            }
            result.references.insert(target, ordered);
        }
        result
    }

    pub fn chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.references.keys().map(|&(x, z)| ChunkPos::new(x, z))
    }

    pub fn place(&mut self, seed: i64, world: &mut impl MineshaftWorld) {
        for (&(x, z), references) in &self.references {
            let pos = ChunkPos::new(x, z);
            let clip = writable_area(pos);
            let mut random = WorldgenRandom::new(seed);
            let decoration_seed = random.set_decoration_seed(seed, x * 16, z * 16);
            for mine_type in [MineType::Normal, MineType::Mesa] {
                random.set_feature_seed(decoration_seed, mine_type.feature_index(), 3);
                for (source, ty) in references {
                    if *ty != mine_type {
                        continue;
                    }
                    let start = self
                        .starts
                        .get_mut(source)
                        .expect("referenced mineshaft start");
                    for piece in &mut start.pieces {
                        if piece.bounds.intersects(clip) {
                            piece.post_process(mine_type, world, &mut random, clip);
                        }
                    }
                }
            }
        }
    }

    pub fn structure_data(&self, pos: ChunkPos) -> StructureData {
        let key = (pos.x, pos.z);
        StructureData {
            mineshaft_start: self.starts.get(&key).cloned(),
            references: self
                .references
                .get(&key)
                .into_iter()
                .flatten()
                .map(|&((x, z), ty)| ([x, z], ty))
                .collect(),
            ..Default::default()
        }
    }
}

fn pack((x, z): Key) -> u64 {
    u64::from(x as u32) | (u64::from(z as u32) << 32)
}

fn mix(key: u64) -> usize {
    let h = key.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let h = h ^ (h >> 32);
    (h ^ (h >> 16)) as usize
}

fn insert(table: &mut [u64], key: u64) {
    let mask = table.len() - 1;
    let mut slot = mix(key) & mask;
    while table[slot] != 0 {
        slot = (slot + 1) & mask;
    }
    table[slot] = key;
}

// LongOpenHashSet's default capacity, linear probing and reverse-slot iterator.
// A references set is filled once in X/Z source order, without removals.
fn reference_order(positions: impl IntoIterator<Item = Key>) -> Vec<Key> {
    let mut table = vec![0; 32];
    let mut seen = BTreeSet::new();
    let mut zero = false;
    for pos in positions {
        if !seen.insert(pos) {
            continue;
        }
        let key = pack(pos);
        if key == 0 {
            zero = true;
        } else {
            insert(&mut table, key);
        }
        if seen.len() > table.len() * 3 / 4 {
            let mut larger = vec![0; table.len() * 2];
            for key in table.into_iter().rev().filter(|&key| key != 0) {
                insert(&mut larger, key);
            }
            table = larger;
        }
    }
    let mut result = Vec::with_capacity(seen.len());
    if zero {
        result.push((0, 0));
    }
    for key in table.into_iter().rev().filter(|&key| key != 0) {
        result.push((key as u32 as i32, (key >> 32) as u32 as i32));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_iterate_in_native_fastutil_order() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../data/mineshaft_starts_26_1.json")).unwrap();
        let rows = fixture["reference_orders"].as_array().unwrap();
        for row in rows {
            let read = |v: &serde_json::Value| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                reference_order(read(&row["inserted"])),
                read(&row["iterated"])
            );
        }
    }
}
