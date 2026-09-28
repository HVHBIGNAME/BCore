//! Mineshaft piece layout, collision checks and vertical placement.
//! Block post-processing is separate from this structure-start graph.
pub mod blocks;
pub mod region;
mod start;

use crate::{simplex::JavaRandom, ChunkPos};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MineType {
    Normal,
    Mesa,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    North,
    South,
    West,
    East,
}

impl Direction {
    fn id(self) -> i32 {
        match self {
            Self::South => 0,
            Self::West => 1,
            Self::North => 2,
            Self::East => 3,
        }
    }
    fn along_z(self) -> bool {
        matches!(self, Self::North | Self::South)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl Bounds {
    fn new(min: [i32; 3], max: [i32; 3]) -> Self {
        Self { min, max }
    }
    pub fn span(self, axis: usize) -> i32 {
        self.max[axis] - self.min[axis] + 1
    }
    pub fn intersects(self, other: Self) -> bool {
        (0..3).all(|i| self.max[i] >= other.min[i] && self.min[i] <= other.max[i])
    }
    pub fn contains(self, pos: [i32; 3]) -> bool {
        (0..3).all(|i| (self.min[i]..=self.max[i]).contains(&pos[i]))
    }
    fn translated(mut self, delta: [i32; 3]) -> Self {
        for (i, d) in delta.into_iter().enumerate() {
            self.min[i] += d;
            self.max[i] += d;
        }
        self
    }
    fn array(self) -> [i32; 6] {
        [
            self.min[0],
            self.min[1],
            self.min[2],
            self.max[0],
            self.max[1],
            self.max[2],
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PieceKind {
    Room {
        entrances: Vec<Bounds>,
    },
    Corridor {
        rails: bool,
        spider: bool,
        placed_spider: bool,
        sections: i32,
    },
    Crossing {
        two_floors: bool,
        direction: Direction,
    },
    Stairs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piece {
    pub bounds: Bounds,
    pub depth: i32,
    pub orientation: Option<Direction>,
    pub kind: PieceKind,
}

impl Piece {
    pub fn save_data(&self, mine_type: MineType) -> Value {
        let mut tag = json!({"BB":self.bounds.array(),"GD":self.depth,"O":self.orientation.map_or(-1,Direction::id),"MST":if mine_type == MineType::Normal {0} else {1}});
        let id = match &self.kind {
            PieceKind::Room { entrances } => {
                tag["Entrances"] = json!(entrances.iter().map(|b| b.array()).collect::<Vec<_>>());
                "msroom"
            }
            PieceKind::Corridor {
                rails,
                spider,
                placed_spider,
                sections,
            } => {
                tag["hr"] = json!(u8::from(*rails));
                tag["sc"] = json!(u8::from(*spider));
                tag["hps"] = json!(u8::from(*placed_spider));
                tag["Num"] = json!(sections);
                "mscorridor"
            }
            PieceKind::Crossing {
                two_floors,
                direction,
            } => {
                tag["tf"] = json!(u8::from(*two_floors));
                tag["D"] = json!(direction.id());
                "mscrossing"
            }
            PieceKind::Stairs => "msstairs",
        };
        tag["id"] = json!(format!("minecraft:{id}"));
        tag
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MineshaftLayout {
    pub mine_type: MineType,
    pub pieces: Vec<Piece>,
}

impl MineshaftLayout {
    /// Structure-specific generation point, after the context's large-feature
    /// seed has been installed. Biome admission and set frequency are separate.
    pub fn start(
        random: &mut JavaRandom,
        source: ChunkPos,
        mine_type: MineType,
        sea_level: i32,
        min_y: i32,
        mut base_height: impl FnMut(i32, i32) -> i32,
    ) -> (Self, [i32; 3]) {
        // Vanilla retains this legacy probability draw even though the
        // structure set now applies the mineshaft frequency separately.
        random.next_double();
        let mut layout = Self::generate(random, source, mine_type);
        let dy = match mine_type {
            MineType::Normal => layout.move_below_sea_level(random, sea_level, min_y, 10),
            MineType::Mesa => {
                let b = layout.bounds();
                let height = base_height(b.min[0] + b.span(0) / 2, b.min[2] + b.span(2) / 2);
                layout.move_mesa_to_surface(random, sea_level, height)
            }
        };
        (layout, [source.x * 16 + 8, 50 + dy, source.z * 16])
    }

    pub fn generate(random: &mut JavaRandom, source: ChunkPos, mine_type: MineType) -> Self {
        let (x, z) = (source.x * 16 + 2, source.z * 16 + 2);
        let bounds = Bounds::new(
            [x, 50, z],
            [
                x + 7 + random.next_int(6) as i32,
                54 + random.next_int(6) as i32,
                z + 7 + random.next_int(6) as i32,
            ],
        );
        let root = Piece {
            bounds,
            depth: 0,
            orientation: None,
            kind: PieceKind::Room {
                entrances: Vec::new(),
            },
        };
        let mut result = Self {
            mine_type,
            pieces: vec![root],
        };
        result.add_children(0, random);
        result
    }

    pub fn bounds(&self) -> Bounds {
        let mut bounds = self.pieces[0].bounds;
        for piece in &self.pieces[1..] {
            for i in 0..3 {
                bounds.min[i] = bounds.min[i].min(piece.bounds.min[i]);
                bounds.max[i] = bounds.max[i].max(piece.bounds.max[i]);
            }
        }
        bounds
    }

    pub fn offset_y(&mut self, dy: i32) {
        for piece in &mut self.pieces {
            piece.bounds = piece.bounds.translated([0, dy, 0]);
            if let PieceKind::Room { entrances } = &mut piece.kind {
                for b in entrances {
                    *b = b.translated([0, dy, 0]);
                }
            }
        }
    }

    pub fn move_below_sea_level(
        &mut self,
        random: &mut JavaRandom,
        sea_level: i32,
        min_y: i32,
        gap: i32,
    ) -> i32 {
        let bounds = self.bounds();
        let mut top = bounds.span(1) + min_y + 1;
        if top < sea_level - gap {
            top += random.next_int((sea_level - gap - top) as usize) as i32;
        }
        let dy = top - bounds.max[1];
        self.offset_y(dy);
        dy
    }

    /// Mesa adjustment uses the terrain's WORLD_SURFACE_WG base height at the
    /// complete layout's horizontal center, before structure block placement.
    pub fn move_mesa_to_surface(
        &mut self,
        random: &mut JavaRandom,
        sea_level: i32,
        base_height: i32,
    ) -> i32 {
        let bounds = self.bounds();
        let center_y = bounds.min[1] + bounds.span(1) / 2;
        let target = if base_height <= sea_level {
            sea_level
        } else {
            sea_level + random.next_int((base_height - sea_level + 1) as usize) as i32
        };
        let dy = target - center_y;
        self.offset_y(dy);
        dy
    }

    fn collides(&self, bounds: Bounds) -> bool {
        self.pieces.iter().any(|p| p.bounds.intersects(bounds))
    }

    fn add_piece(
        &mut self,
        random: &mut JavaRandom,
        pos: [i32; 3],
        direction: Direction,
        parent_depth: i32,
    ) -> Option<usize> {
        let root = self.pieces[0].bounds;
        if parent_depth > 8
            || (pos[0] - root.min[0]).abs() > 80
            || (pos[2] - root.min[2]).abs() > 80
        {
            return None;
        }
        let roll = random.next_int(100);
        let (bounds, kind, orientation) = if roll >= 80 {
            let top = if random.next_int(4) == 0 { 6 } else { 2 };
            let bounds = match direction {
                Direction::North => Bounds::new([-1, 0, -4], [3, top, 0]),
                Direction::South => Bounds::new([-1, 0, 0], [3, top, 4]),
                Direction::West => Bounds::new([-4, 0, -1], [0, top, 3]),
                Direction::East => Bounds::new([0, 0, -1], [4, top, 3]),
            }
            .translated(pos);
            if self.collides(bounds) {
                return None;
            }
            (
                bounds,
                PieceKind::Crossing {
                    two_floors: top > 2,
                    direction,
                },
                None,
            )
        } else if roll >= 70 {
            let bounds = Self::shaft_bounds(direction, 9, -5).translated(pos);
            if self.collides(bounds) {
                return None;
            }
            (bounds, PieceKind::Stairs, Some(direction))
        } else {
            let mut found = None;
            for sections in (1..=random.next_int(3) as i32 + 2).rev() {
                let bounds = Self::shaft_bounds(direction, sections * 5, 0).translated(pos);
                if !self.collides(bounds) {
                    found = Some((bounds, sections));
                    break;
                }
            }
            let (bounds, sections) = found?;
            let rails = random.next_int(3) == 0;
            let spider = !rails && random.next_int(23) == 0;
            (
                bounds,
                PieceKind::Corridor {
                    rails,
                    spider,
                    placed_spider: false,
                    sections,
                },
                Some(direction),
            )
        };
        let index = self.pieces.len();
        self.pieces.push(Piece {
            bounds,
            kind,
            orientation,
            depth: parent_depth + 1,
        });
        self.add_children(index, random);
        Some(index)
    }

    fn shaft_bounds(direction: Direction, length: i32, bottom: i32) -> Bounds {
        match direction {
            Direction::North => Bounds::new([0, bottom, 1 - length], [2, 2, 0]),
            Direction::South => Bounds::new([0, bottom, 0], [2, 2, length - 1]),
            Direction::West => Bounds::new([1 - length, bottom, 0], [0, 2, 2]),
            Direction::East => Bounds::new([0, bottom, 0], [length - 1, 2, 2]),
        }
    }

    fn add_children(&mut self, index: usize, random: &mut JavaRandom) {
        let piece = self.pieces[index].clone();
        match piece.kind {
            PieceKind::Room { .. } => self.room_children(index, random, piece.bounds),
            PieceKind::Corridor { .. } => self.corridor_children(random, &piece),
            PieceKind::Stairs => {
                let dir = piece.orientation.unwrap();
                self.add_piece(random, Self::forward(piece.bounds, dir), dir, piece.depth);
            }
            PieceKind::Crossing {
                direction,
                two_floors,
            } => self.crossing_children(random, piece.bounds, direction, two_floors, piece.depth),
        }
    }

    fn forward(b: Bounds, dir: Direction) -> [i32; 3] {
        match dir {
            Direction::North => [b.min[0], b.min[1], b.min[2] - 1],
            Direction::South => [b.min[0], b.min[1], b.max[2] + 1],
            Direction::West => [b.min[0] - 1, b.min[1], b.min[2]],
            Direction::East => [b.max[0] + 1, b.min[1], b.min[2]],
        }
    }

    fn room_children(&mut self, index: usize, random: &mut JavaRandom, b: Bounds) {
        for dir in [
            Direction::North,
            Direction::South,
            Direction::West,
            Direction::East,
        ] {
            let span = b.span(if dir.along_z() { 0 } else { 2 });
            let mut offset = 0;
            while offset < span {
                offset += random.next_int(span as usize) as i32;
                if offset + 3 > span {
                    break;
                }
                let mut pos = Self::forward(b, dir);
                pos[if dir.along_z() { 0 } else { 2 }] += offset;
                pos[1] += 1 + random.next_int((b.span(1) - 4).max(1) as usize) as i32;
                if let Some(child) = self.add_piece(random, pos, dir, 0) {
                    let mut entrance = self.pieces[child].bounds;
                    match dir {
                        Direction::North => {
                            entrance.min[2] = b.min[2];
                            entrance.max[2] = b.min[2] + 1;
                        }
                        Direction::South => {
                            entrance.min[2] = b.max[2] - 1;
                            entrance.max[2] = b.max[2];
                        }
                        Direction::West => {
                            entrance.min[0] = b.min[0];
                            entrance.max[0] = b.min[0] + 1;
                        }
                        Direction::East => {
                            entrance.min[0] = b.max[0] - 1;
                            entrance.max[0] = b.max[0];
                        }
                    }
                    if let PieceKind::Room { entrances } = &mut self.pieces[index].kind {
                        entrances.push(entrance);
                    }
                }
                offset += 4;
            }
        }
    }

    fn corridor_children(&mut self, random: &mut JavaRandom, piece: &Piece) {
        let b = piece.bounds;
        let dir = piece.orientation.unwrap();
        let choice = random.next_int(4);
        let (mut pos, next_dir) = if choice <= 1 {
            (Self::forward(b, dir), dir)
        } else {
            match (dir, choice) {
                (Direction::North, 2) => ([b.min[0] - 1, b.min[1], b.min[2]], Direction::West),
                (Direction::North, _) => ([b.max[0] + 1, b.min[1], b.min[2]], Direction::East),
                (Direction::South, 2) => ([b.min[0] - 1, b.min[1], b.max[2] - 3], Direction::West),
                (Direction::South, _) => ([b.max[0] + 1, b.min[1], b.max[2] - 3], Direction::East),
                (Direction::West, 2) => ([b.min[0], b.min[1], b.min[2] - 1], Direction::North),
                (Direction::West, _) => ([b.min[0], b.min[1], b.max[2] + 1], Direction::South),
                (Direction::East, 2) => ([b.max[0] - 3, b.min[1], b.min[2] - 1], Direction::North),
                (Direction::East, _) => ([b.max[0] - 3, b.min[1], b.max[2] + 1], Direction::South),
            }
        };
        pos[1] += random.next_int(3) as i32 - 1;
        self.add_piece(random, pos, next_dir, piece.depth);
        if piece.depth >= 8 {
            return;
        }
        let axis = if dir.along_z() { 2 } else { 0 };
        let mut at = b.min[axis] + 3;
        while at + 3 <= b.max[axis] {
            let side = random.next_int(5);
            if side <= 1 {
                let (pos, dir) = if axis == 2 {
                    (
                        [
                            if side == 0 {
                                b.min[0] - 1
                            } else {
                                b.max[0] + 1
                            },
                            b.min[1],
                            at,
                        ],
                        if side == 0 {
                            Direction::West
                        } else {
                            Direction::East
                        },
                    )
                } else {
                    (
                        [
                            at,
                            b.min[1],
                            if side == 0 {
                                b.min[2] - 1
                            } else {
                                b.max[2] + 1
                            },
                        ],
                        if side == 0 {
                            Direction::North
                        } else {
                            Direction::South
                        },
                    )
                };
                self.add_piece(random, pos, dir, piece.depth + 1);
            }
            at += 5;
        }
    }

    fn crossing_children(
        &mut self,
        random: &mut JavaRandom,
        b: Bounds,
        direction: Direction,
        two_floors: bool,
        depth: i32,
    ) {
        let exits = match direction {
            Direction::North => [Direction::North, Direction::West, Direction::East],
            Direction::South => [Direction::South, Direction::West, Direction::East],
            Direction::West => [Direction::North, Direction::South, Direction::West],
            Direction::East => [Direction::North, Direction::South, Direction::East],
        };
        let position = |dir: Direction, dy| {
            let mut pos = Self::forward(b, dir);
            pos[if dir.along_z() { 0 } else { 2 }] += 1;
            pos[1] += dy;
            pos
        };
        for dir in exits {
            self.add_piece(random, position(dir, 0), dir, depth);
        }
        if two_floors {
            for dir in [
                Direction::North,
                Direction::West,
                Direction::East,
                Direction::South,
            ] {
                if random.next_bool() {
                    self.add_piece(random, position(dir, 4), dir, depth);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_piece_graphs_match_native_26_1() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../data/mineshafts_26_1.json")).unwrap();
        for sample in fixture["samples"].as_array().unwrap() {
            let seed = sample["seed"].as_i64().unwrap();
            let source = ChunkPos::new(
                sample["chunk"][0].as_i64().unwrap() as i32,
                sample["chunk"][1].as_i64().unwrap() as i32,
            );
            let kind = if sample["type"] == "NORMAL" {
                MineType::Normal
            } else {
                MineType::Mesa
            };
            let mut random = crate::structure::placement::large_feature_random(seed, source);
            let (layout, point) =
                MineshaftLayout::start(&mut random, source, kind, 63, -64, |x, z| {
                    assert_eq!(json!([x, z]), sample["base_height_position"]);
                    sample["base_height"].as_i64().unwrap() as i32
                });
            let expected = sample["pieces"].as_array().unwrap();
            assert_eq!(
                layout.pieces.len(),
                expected.len(),
                "seed {seed} {source:?} {kind:?}"
            );
            for (i, (piece, expected)) in layout.pieces.iter().zip(expected).enumerate() {
                assert_eq!(
                    piece.save_data(kind),
                    *expected,
                    "seed {seed} {source:?} {kind:?} piece {i}"
                );
            }
            assert_eq!(json!(point), sample["generation_point"]);
            assert_eq!(
                (point[1] - 50) as i64,
                sample["vertical_offset"].as_i64().unwrap()
            );
            assert_eq!(
                random.next_long(),
                sample["next_i64"].as_i64().unwrap(),
                "seed {seed} {source:?} {kind:?}"
            );
        }
    }
}
