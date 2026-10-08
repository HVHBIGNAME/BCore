//! 26.1 desert-pyramid geometry, cellar and `Structure.afterPlace` archaeology.
//!
//! The feature RNG, source-region RNG and positional legacy streams are distinct.
//! The scheduler retains the piece across clips; native save/reload clears only
//! transient archaeology candidates, not HPos or the four chest placement flags.

use super::*;
use crate::noise_perlin::Xoroshiro;
use crate::simplex::JavaRandom;
use crate::structure::template::shuffle;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchaeologyState {
    /// Native append-only list, including duplicates across piece invocations.
    pub potential: Vec<Pos>,
    /// Native constructor sentinel is BlockPos.ZERO, even before postProcess.
    pub roof: Pos,
}

/// The fresh `WorldGenRegion` stream for an overworld source task. Call once per
/// stage, then pass the continuing stream through every consumer in source order.
pub fn region_random(world_seed: i64, source: ChunkPos) -> Xoroshiro {
    Xoroshiro::new(world_seed)
        .fork_positional()
        .from_hash_of("minecraft:worldgen_region_random")
        .fork_positional()
        .at(source.x.wrapping_mul(16), 0, source.z.wrapping_mul(16))
}

/// Real source-clipped placement followed by archaeology, including when the
/// current piece box misses the clip. Does not reseed either caller-owned RNG.
pub fn place_in_chunk<W: ScatteredWorld + ?Sized, R: TemplateRandom + ?Sized>(
    start: &mut ScatteredStart,
    world: &mut W,
    random: &mut R,
    region_random: &mut Xoroshiro,
    world_seed: i64,
    clip: BoundingBox,
) -> Result<ScatteredPlacement> {
    if start.kind != ScatteredKind::DesertPyramid
        || start.piece.kind() != start.kind
        || !start.piece.valid()
    {
        return Err(invalid("desert-pyramid piece before placement"));
    }
    let intersects = start.piece.bounds.intersects(clip);
    let mut report = ScatteredPlacement::new(if intersects {
        ScatteredStatus::Processed
    } else {
        ScatteredStatus::OutsideClip
    });
    let mut placer = Placer {
        world,
        catalog: ScatteredCatalog::bundled(),
        blocks: &StructureAssets::bundled().blocks,
        clip,
        report: &mut report,
    };
    if intersects {
        placer.desert_pyramid(&mut start.piece, random, region_random, world_seed)?;
    }
    placer.pyramid_after_place(&start.piece, world_seed)?;
    Ok(report)
}

fn positional_random(seed: i64, p: Pos) -> JavaRandom {
    JavaRandom::new(JavaRandom::new(seed).next_long() ^ crate::random::get_seed(p.0, p.1, p.2))
}

fn archaeology(piece: &ScatteredPiece) -> &ArchaeologyState {
    let ScatteredPieceData::DesertPyramid { archaeology, .. } = &piece.data else {
        unreachable!()
    };
    archaeology
}

fn archaeology_mut(piece: &mut ScatteredPiece) -> &mut ArchaeologyState {
    let ScatteredPieceData::DesertPyramid { archaeology, .. } = &mut piece.data else {
        unreachable!()
    };
    archaeology
}

impl<W: ScatteredWorld + ?Sized> Placer<'_, W> {
    fn pyramid_box(
        &mut self,
        piece: &ScatteredPiece,
        min: Pos,
        max: Pos,
        outside: u32,
        inside: u32,
        skip_air: bool,
    ) -> Result<()> {
        for y in min.1..=max.1 {
            for x in min.0..=max.0 {
                for z in min.2..=max.2 {
                    let local = (x, y, z);
                    if skip_air {
                        let p = piece.world_pos(local);
                        // StructurePiece.getBlock returns AIR outside its clip.
                        if !self.clip.contains(p)
                            || self.catalog.block_properties(read(self.world, p)?)?.air
                        {
                            continue;
                        }
                    }
                    let boundary = y == min.1
                        || y == max.1
                        || x == min.0
                        || x == max.0
                        || z == min.2
                        || z == max.2;
                    self.piece_block(piece, local, if boundary { outside } else { inside })?;
                }
            }
        }
        Ok(())
    }

    fn desert_pyramid(
        &mut self,
        piece: &mut ScatteredPiece,
        random: &mut (impl TemplateRandom + ?Sized),
        region_random: &mut Xoroshiro,
        world_seed: i64,
    ) -> Result<()> {
        // Drawn even when HPos is already cached, before the ground helper's gate.
        let offset = -(random.next_int(3) as i32);
        let ScatteredPieceData::DesertPyramid {
            height_position, ..
        } = &mut piece.data
        else {
            unreachable!()
        };
        if *height_position < 0 {
            let mut lowest = crate::MAX_Y + 1;
            // Full footprint, Z-major, independent of the current source clip.
            for z in piece.bounds.min.2..=piece.bounds.max.2 {
                for x in piece.bounds.min.0..=piece.bounds.max.0 {
                    lowest = lowest.min(self.world.feature_height(
                        FeatureHeightmap::MotionBlockingNoLeaves,
                        x,
                        z,
                    ));
                }
            }
            *height_position = lowest;
            piece.bounds = piece.bounds.moved((
                0,
                lowest.wrapping_sub(piece.bounds.min.1).wrapping_add(offset),
                0,
            ));
        }
        let sandstone = self.blocks.default_state("sandstone")?;
        let cut = self.blocks.default_state("cut_sandstone")?;
        let chiseled = self.blocks.default_state("chiseled_sandstone")?;
        let air = self.blocks.default_state("air")?;
        let orange = self.blocks.default_state("orange_terracotta")?;
        let blue = self.blocks.default_state("blue_terracotta")?;
        self.box_fill(piece, (0, -4, 0), (20, 0, 20), sandstone)?;
        for level in 1..=9 {
            self.box_fill(
                piece,
                (level, level, level),
                (20 - level, level, 20 - level),
                sandstone,
            )?;
            self.box_fill(
                piece,
                (level + 1, level, level + 1),
                (19 - level, level, 19 - level),
                air,
            )?;
        }
        for x in 0..21 {
            for z in 0..21 {
                self.column_down(piece, (x, -5, z), sandstone)?;
            }
        }
        let stairs = self.blocks.default_state("sandstone_stairs")?;
        let north = self.blocks.with_property(stairs, "facing", "north")?;
        let south = self.blocks.with_property(stairs, "facing", "south")?;
        let east = self.blocks.with_property(stairs, "facing", "east")?;
        let west = self.blocks.with_property(stairs, "facing", "west")?;
        for x in [0, 16] {
            self.pyramid_box(piece, (x, 0, 0), (x + 4, 9, 4), sandstone, air, false)?;
            self.box_fill(piece, (x + 1, 10, 1), (x + 3, 10, 3), sandstone)?;
            for (p, state) in [
                ((x + 2, 10, 0), north),
                ((x + 2, 10, 4), south),
                ((x, 10, 2), east),
                ((x + 4, 10, 2), west),
            ] {
                self.piece_block(piece, p, state)?;
            }
        }
        self.pyramid_box(piece, (8, 0, 0), (12, 4, 4), sandstone, air, false)?;
        self.box_fill(piece, (9, 1, 0), (11, 3, 4), air)?;
        for p in [
            (9, 1, 1),
            (9, 2, 1),
            (9, 3, 1),
            (10, 3, 1),
            (11, 3, 1),
            (11, 2, 1),
            (11, 1, 1),
        ] {
            self.piece_block(piece, p, cut)?;
        }
        for x in [4, 12] {
            self.pyramid_box(piece, (x, 1, 1), (x + 4, 3, 3), sandstone, air, false)?;
            self.box_fill(piece, (x, 1, 2), (x + 4, 2, 2), air)?;
        }
        self.box_fill(piece, (5, 4, 5), (15, 4, 15), sandstone)?;
        self.box_fill(piece, (9, 4, 9), (11, 4, 11), air)?;
        for (x, z) in [(8, 8), (12, 8), (8, 12), (12, 12)] {
            self.box_fill(piece, (x, 1, z), (x, 3, z), cut)?;
        }
        for (min, max, state) in [
            ((1, 1, 5), (4, 4, 11), sandstone),
            ((16, 1, 5), (19, 4, 11), sandstone),
            ((6, 7, 9), (6, 7, 11), sandstone),
            ((14, 7, 9), (14, 7, 11), sandstone),
            ((5, 5, 9), (5, 7, 11), cut),
            ((15, 5, 9), (15, 7, 11), cut),
        ] {
            self.box_fill(piece, min, max, state)?;
        }
        for p in [
            (5, 5, 10),
            (5, 6, 10),
            (6, 6, 10),
            (15, 5, 10),
            (15, 6, 10),
            (14, 6, 10),
        ] {
            self.piece_block(piece, p, air)?;
        }
        self.box_fill(piece, (2, 4, 4), (2, 6, 4), air)?;
        self.box_fill(piece, (18, 4, 4), (18, 6, 4), air)?;
        for p in [(2, 4, 5), (2, 3, 4), (18, 4, 5), (18, 3, 4)] {
            self.piece_block(piece, p, north)?;
        }
        self.box_fill(piece, (1, 1, 3), (2, 2, 3), sandstone)?;
        self.box_fill(piece, (18, 1, 3), (19, 2, 3), sandstone)?;
        for p in [(1, 1, 2), (19, 1, 2)] {
            self.piece_block(piece, p, sandstone)?;
        }
        let slab = self.blocks.default_state("sandstone_slab")?;
        for p in [(1, 2, 2), (19, 2, 2)] {
            self.piece_block(piece, p, slab)?;
        }
        self.piece_block(piece, (2, 1, 2), west)?;
        self.piece_block(piece, (18, 1, 2), east)?;
        self.box_fill(piece, (4, 3, 5), (4, 3, 17), sandstone)?;
        self.box_fill(piece, (16, 3, 5), (16, 3, 17), sandstone)?;
        self.box_fill(piece, (3, 1, 5), (4, 2, 16), air)?;
        self.box_fill(piece, (15, 1, 5), (16, 2, 16), air)?;
        for z in (5..=17).step_by(2) {
            for (p, state) in [
                ((4, 1, z), cut),
                ((4, 2, z), chiseled),
                ((16, 1, z), cut),
                ((16, 2, z), chiseled),
            ] {
                self.piece_block(piece, p, state)?;
            }
        }
        for (x, z) in [
            (10, 7),
            (10, 8),
            (9, 9),
            (11, 9),
            (8, 10),
            (12, 10),
            (7, 10),
            (13, 10),
            (9, 11),
            (11, 11),
            (10, 12),
            (10, 13),
        ] {
            self.piece_block(piece, (x, 0, z), orange)?;
        }
        self.piece_block(piece, (10, 0, 10), blue)?;
        let glyph = [
            [cut, orange, cut],
            [cut, orange, cut],
            [orange, chiseled, orange],
            [cut, orange, cut],
            [orange, chiseled, orange],
            [orange, orange, orange],
            [cut, cut, cut],
        ];
        for x in [0, 20] {
            for (row, states) in glyph.iter().enumerate() {
                for (column, &state) in states.iter().enumerate() {
                    self.piece_block(piece, (x, row as i32 + 2, column as i32 + 1), state)?;
                }
            }
        }
        for x in [2, 18] {
            for (row, states) in glyph.iter().enumerate() {
                for (column, &state) in states.iter().enumerate() {
                    self.piece_block(piece, (x + column as i32 - 1, row as i32 + 2, 0), state)?;
                }
            }
        }
        self.box_fill(piece, (8, 4, 0), (12, 6, 0), cut)?;
        for (p, state) in [
            ((8, 6, 0), air),
            ((12, 6, 0), air),
            ((9, 5, 0), orange),
            ((10, 5, 0), chiseled),
            ((11, 5, 0), orange),
        ] {
            self.piece_block(piece, p, state)?;
        }
        for (min, max, state) in [
            ((8, -14, 8), (12, -11, 12), cut),
            ((8, -10, 8), (12, -10, 12), chiseled),
            ((8, -9, 8), (12, -9, 12), cut),
            ((8, -8, 8), (12, -1, 12), sandstone),
            ((9, -11, 9), (11, -1, 11), air),
        ] {
            self.box_fill(piece, min, max, state)?;
        }
        self.piece_block(
            piece,
            (10, -11, 10),
            self.blocks.default_state("stone_pressure_plate")?,
        )?;
        self.box_fill(
            piece,
            (9, -13, 9),
            (11, -13, 11),
            self.blocks.default_state("tnt")?,
        )?;
        for (inner, outer) in [
            ((8, 10), (7, 10)),
            ((12, 10), (13, 10)),
            ((10, 8), (10, 7)),
            ((10, 12), (10, 13)),
        ] {
            self.piece_block(piece, (inner.0, -11, inner.1), air)?;
            self.piece_block(piece, (inner.0, -10, inner.1), air)?;
            self.piece_block(piece, (outer.0, -10, outer.1), chiseled)?;
            self.piece_block(piece, (outer.0, -11, outer.1), cut)?;
        }
        for direction in HorizontalDirection::ALL {
            let index = direction.data_value() as usize;
            let ScatteredPieceData::DesertPyramid {
                has_placed_chest, ..
            } = &piece.data
            else {
                unreachable!()
            };
            if !has_placed_chest[index] {
                let d = direction.step();
                let success = self.chest_at(
                    piece.world_pos((10 + d.0 * 2, -11, 10 + d.2 * 2)),
                    "minecraft:chests/desert_pyramid",
                    random,
                )?;
                let ScatteredPieceData::DesertPyramid {
                    has_placed_chest, ..
                } = &mut piece.data
                else {
                    unreachable!()
                };
                has_placed_chest[index] = success;
            }
        }
        self.pyramid_cellar(piece, region_random, world_seed)
    }

    fn pyramid_sand(piece: &mut ScatteredPiece, p: Pos) {
        let p = piece.world_pos(p);
        archaeology_mut(piece).potential.push(p);
    }

    fn pyramid_cellar(
        &mut self,
        piece: &mut ScatteredPiece,
        region_random: &mut Xoroshiro,
        world_seed: i64,
    ) -> Result<()> {
        let sand = self.blocks.default_state("sand")?;
        let sandstone = self.blocks.default_state("sandstone")?;
        let cut = self.blocks.default_state("cut_sandstone")?;
        let chiseled = self.blocks.default_state("chiseled_sandstone")?;
        let orange = self.blocks.default_state("orange_terracotta")?;
        let stairs = self.blocks.transform(
            self.blocks.default_state("sandstone_stairs")?,
            Mirror::None,
            Rotation::Counterclockwise90,
        )?;
        for p in [(13, -1, 17), (14, -2, 17), (15, -3, 17)] {
            self.piece_block(piece, p, stairs)?;
        }
        // Bare XoroshiroRandomSource.nextBoolean uses its low bit, not the
        // WorldgenRandom.next(1) wrapper or a bounded nextInt(2).
        let rubble = region_random.next_long() & 1 != 0;
        for p in [
            (12, 0, 17),
            (13, 0, 17),
            (14, 0, 17),
            (15, 0, 17),
            (16, 0, 17),
            (14, -1, 17),
        ] {
            self.piece_block(piece, p, sand)?;
        }
        self.piece_block(piece, (15, -1, 17), if rubble { sand } else { sandstone })?;
        self.piece_block(piece, (16, -1, 17), if rubble { sandstone } else { sand })?;
        self.piece_block(piece, (15, -2, 17), sand)?;
        self.piece_block(piece, (16, -2, 17), sandstone)?;
        self.piece_block(piece, (16, -3, 17), sand)?;
        for (y, state) in [(-3, cut), (-2, chiseled), (-1, cut)] {
            for (min, max) in [
                ((13, y, 10), (13, y, 15)),
                ((19, y, 10), (19, y, 15)),
                ((13, y, 10), (19, y, 11)),
                ((13, y, 16), (19, y, 16)),
            ] {
                self.pyramid_box(piece, min, max, state, state, true)?;
            }
        }
        for y in -3..=-1 {
            for x in 14..=18 {
                for z in 11..=15 {
                    Self::pyramid_sand(piece, (x, y, z));
                }
            }
        }
        for x in 14..=18 {
            for z in 11..=15 {
                let state = if region_random.next_float() < 0.33_f32 {
                    sandstone
                } else {
                    sand
                };
                self.piece_block(piece, (x, 0, z), state)?;
            }
        }
        let mut positional = positional_random(world_seed, piece.world_pos((14, 0, 11)));
        let roof = piece.world_pos((
            14 + positional.next_int(5) as i32,
            0,
            11 + positional.next_int(5) as i32,
        ));
        archaeology_mut(piece).roof = roof;
        self.piece_block(
            piece,
            (16, -4, 13),
            self.blocks.default_state("blue_terracotta")?,
        )?;
        for p in [
            (17, -4, 12),
            (17, -4, 14),
            (15, -4, 12),
            (15, -4, 14),
            (18, -4, 13),
            (14, -4, 13),
            (16, -4, 15),
            (16, -4, 11),
        ] {
            self.piece_block(piece, p, orange)?;
        }
        for x in [19, 13] {
            self.piece_block(piece, (x, -4, 13), orange)?;
            Self::pyramid_sand(piece, (x, -3, 13));
            Self::pyramid_sand(piece, (x, -2, 13));
            let outside = if x == 19 { 20 } else { 12 };
            self.piece_block(piece, (outside, -3, 13), cut)?;
            self.piece_block(piece, (outside, -2, 13), chiseled)?;
        }
        self.piece_block(piece, (16, -4, 16), orange)?;
        Self::pyramid_sand(piece, (16, -3, 16));
        Self::pyramid_sand(piece, (16, -2, 16));
        self.piece_block(piece, (16, -4, 10), orange)?;
        Self::pyramid_sand(piece, (16, -3, 10));
        Self::pyramid_sand(piece, (16, -2, 10));
        self.piece_block(piece, (16, -3, 9), cut)?;
        self.piece_block(piece, (16, -2, 9), chiseled)?;
        Ok(())
    }

    fn suspicious_sand(&mut self, p: Pos) -> Result<()> {
        if self.clip.contains(p) {
            self.write(p, self.blocks.default_state("suspicious_sand")?, 2);
            if self
                .world
                .has_structure_block_entity(p, "minecraft:brushable_block")
            {
                // BlockPos.asLong; no draw from either caller-owned stream.
                let packed = ((p.0 as i64 & 0x3ff_ffff) << 38)
                    | ((p.2 as i64 & 0x3ff_ffff) << 12)
                    | (p.1 as i64 & 0xfff);
                self.world
                    .apply_scattered_effect(ScatteredEffect::LootTable {
                        pos: p,
                        block_entity: "minecraft:brushable_block".into(),
                        table: "minecraft:archaeology/desert_pyramid".into(),
                        seed: packed,
                    })?;
                self.report.loot_assignments += 1;
            }
        }
        Ok(())
    }

    fn pyramid_after_place(&mut self, piece: &ScatteredPiece, world_seed: i64) -> Result<()> {
        let archaeology = archaeology(piece);
        self.suspicious_sand(archaeology.roof)?;
        // SortedArraySet(BlockPos::compareTo): Y, then Z, then X, followed by
        // native Fisher-Yates. Deduplication is before shuffle and clipping.
        let mut positions = archaeology.potential.clone();
        positions.sort_unstable_by_key(|p| (p.1, p.2, p.0));
        positions.dedup();
        let b = piece.bounds;
        let center = (
            b.min.0 + (b.max.0 - b.min.0 + 1) / 2,
            b.min.1 + (b.max.1 - b.min.1 + 1) / 2,
            b.min.2 + (b.max.2 - b.min.2 + 1) / 2,
        );
        let mut random = positional_random(world_seed, center);
        shuffle(&mut positions, &mut random);
        let count = positions.len().min(5 + random.next_int(3));
        let sand = self.blocks.default_state("sand")?;
        for (index, p) in positions.into_iter().enumerate() {
            if index < count {
                self.suspicious_sand(p)?;
            } else if self.clip.contains(p) {
                self.write(p, sand, 2);
            }
        }
        Ok(())
    }
}
