//! Real pre-feature stage boundaries. Metadata and feature effects survive each pass.
use rayon::prelude::*;

use crate::{density, GeneratedChunk, VanillaGraph, WorldGenerator, MAX_Y, MIN_Y};

struct DensityScope;
impl DensityScope {
    fn new() -> Self {
        density::clear_density_caches();
        Self
    }
}
impl Drop for DensityScope {
    fn drop(&mut self) {
        density::clear_density_caches();
    }
}

impl WorldGenerator {
    fn chunk_context(self, pos: crate::ChunkPos) -> density::EvalContext {
        density::EvalContext {
            seed: self.seed(),
            ..Default::default()
        }
        .with_noise_bounds(pos.x * 16, pos.z * 16, 4)
    }

    pub(crate) fn generate_biomes(self, chunk: &mut GeneratedChunk, graph: &VanillaGraph) {
        let _scope = DensityScope::new();
        let ctx = self.chunk_context(chunk.pos);
        let mut cells = vec![0; 1536];
        // ChunkAccess visits ascending sections; LevelChunkSection then visits
        // X/Y/Z. Climate.RTree retains the last winner on ties, so query order
        // is observable and must not be replaced by the palette's Y/Z/X order.
        for section_y in MIN_Y / 16..=MAX_Y / 16 {
            for qx in 0..4 {
                for local_y in 0..4 {
                    let qy = section_y * 4 + local_y;
                    for qz in 0..4 {
                        let index = ((qy - MIN_Y / 4) * 16 + qz * 4 + qx) as usize;
                        cells[index] = graph.noise_biome_at(
                            chunk.pos.x * 4 + qx,
                            qy,
                            chunk.pos.z * 4 + qz,
                            &ctx,
                        );
                    }
                }
            }
        }
        chunk.noise_biomes = Some(cells);
    }

    pub(crate) fn generate_noise(self, chunk: &mut GeneratedChunk, graph: &VanillaGraph) {
        self.generate_noise_with_structures(
            chunk,
            graph,
            &crate::beardifier::Beardifier::default(),
        );
    }

    /// NOISE material fill for the target chunk's admitted structure references.
    /// StructureManager traversal order is retained by the supplied Beardifier.
    pub(crate) fn generate_noise_with_structures(
        self,
        chunk: &mut GeneratedChunk,
        graph: &VanillaGraph,
        beardifier: &crate::beardifier::Beardifier,
    ) {
        let _scope = DensityScope::new();
        let ctx = self.chunk_context(chunk.pos);
        let (base_x, base_z) = (chunk.pos.x * 16, chunk.pos.z * 16);
        let columns: Vec<_> = (0..256)
            .into_par_iter()
            // Reuse corners across the columns consumed by one Rayon job. Its
            // graph and full EvalContext stay fixed; both scope boundaries clear
            // the worker's caches before another chunk/context can use them.
            .map_init(DensityScope::new, |_, index| {
                Self::build_noise_column_with_structures(
                    self.seed(),
                    graph,
                    &ctx,
                    base_x + (index % 16) as i32,
                    base_z + (index / 16) as i32,
                    beardifier,
                )
            })
            .collect();
        let marks_start = chunk.postprocessing.len();
        for (index, column) in columns.into_iter().enumerate() {
            chunk.heights[index] = column.top;
            chunk.biomes[index] = column.biome;
            for (dy, state) in column.states.into_iter().enumerate() {
                chunk.states[dy * 256 + index] = state;
            }
            chunk.postprocessing.extend(
                column
                    .fluid_postprocessing
                    .into_iter()
                    .map(|y| (index % 16, y, index / 16)),
            );
        }
        // Rayon columns do not define the side-effect order. Native NOISE visits
        // cell X, cell Z, descending Y, in-cell X, then in-cell Z. Only this
        // pass's new marks are ordered; previous stage marks retain their order.
        chunk.postprocessing[marks_start..]
            .sort_by_key(|&(x, y, z)| (x / 4, z / 4, std::cmp::Reverse(y), x % 4, z % 4));
    }

    #[cfg(test)]
    pub(crate) fn generate_surface(self, chunk: &mut GeneratedChunk, graph: &VanillaGraph) {
        let ctx = self.chunk_context(chunk.pos);
        self.generate_surface_with_biomes(chunk, graph, |_, qx, qy, qz| {
            graph.noise_biome_at(qx, qy, qz, &ctx)
        });
    }

    pub(crate) fn generate_surface_with_biomes(
        self,
        chunk: &mut GeneratedChunk,
        graph: &VanillaGraph,
        biomes: impl FnMut(&GeneratedChunk, i32, i32, i32) -> crate::biome::BiomeId,
    ) {
        let _scope = DensityScope::new();
        let ctx = self.chunk_context(chunk.pos);
        let preliminary = graph
            .preliminary_surface_level
            .as_ref()
            .expect("surface preliminary density");
        let marks = crate::surface_builder::build_surface(
            chunk,
            self.seed(),
            graph
                .surface_rule
                .as_ref()
                .expect("overworld surface rules"),
            density::noise_registry(),
            |x, z| density::evaluate(preliminary, x as f64, 0.0, z as f64, &ctx).floor() as i32,
            biomes,
        );
        chunk.postprocessing.extend(marks);
    }

    pub(crate) fn generate_carvers(self, chunk: &mut GeneratedChunk, graph: &VanillaGraph) {
        let _scope = DensityScope::new();
        crate::carver::apply(
            self.seed(),
            chunk.pos,
            chunk,
            graph,
            self.chunk_context(chunk.pos),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChunkPos;

    #[test]
    fn native_chunk_biome_fill_matches_order_dependent_palette_histories() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../data/biome_fill_26_1.json")).unwrap();
        assert_eq!(
            fixture["jar_sha256"],
            crate::structure::template_pool::StructureAssets::JAR_SHA256
        );
        let mut count = 0;
        for history in fixture["histories"].as_array().unwrap() {
            let seed = history["seed"].as_i64().unwrap();
            let generator = WorldGenerator::new(seed);
            let graph = VanillaGraph::load().unwrap().fork();
            for row in history["chunks"].as_array().unwrap() {
                let pos = ChunkPos::new(
                    row["chunk"][0].as_i64().unwrap() as i32,
                    row["chunk"][1].as_i64().unwrap() as i32,
                );
                let mut chunk = GeneratedChunk::new(pos);
                generator.generate_biomes(&mut chunk, &graph);
                let actual: Vec<_> = chunk
                    .noise_biomes
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|&id| format!("minecraft:{}", crate::biome::name(id)))
                    .collect();
                let expected: Vec<String> =
                    serde_json::from_value(row["biomes_yzx"].clone()).unwrap();
                assert_eq!(
                    actual, expected,
                    "seed {seed}, chunk {pos:?}, native query history"
                );
                count += 1;
            }
        }
        assert_eq!(count, 12);
    }

    #[test]
    fn noise_jobs_match_cold_columns_across_seeds_and_bounds() {
        // One worker gives both implementations the same ordered biome-query
        // history. Each graph keeps its own live ParameterList throughout.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            let graphs: Vec<_> = (0..3)
                .map(|_| {
                    let graph = VanillaGraph::load().unwrap();
                    (graph.fork(), graph.fork())
                })
                .collect();
            let mut first_seed_one: Option<GeneratedChunk> = None;
            for (world, seed, pos) in [
                (0, 1234, ChunkPos::new(-34, 3)),
                (1, 1, ChunkPos::new(0, 0)),
                (2, 2, ChunkPos::new(0, 0)),
                (0, 1234, ChunkPos::new(1, -1)),
                (1, 1, ChunkPos::new(0, 0)),
            ] {
                let generator = WorldGenerator::new(seed);
                let (graph, reference) = &graphs[world];
                let mut actual = GeneratedChunk::new(pos);
                let mut expected = GeneratedChunk::new(pos);
                generator.generate_biomes(&mut actual, graph);
                generator.generate_biomes(&mut expected, reference);
                generator.generate_noise(&mut actual, graph);
                assert_eq!(density::density_cache_entries(), [0; 5]);
                let ctx = generator.chunk_context(pos);
                for index in 0..256 {
                    let _scope = DensityScope::new();
                    let column = WorldGenerator::build_noise_column(
                        seed,
                        reference,
                        &ctx,
                        pos.x * 16 + (index % 16) as i32,
                        pos.z * 16 + (index / 16) as i32,
                    );
                    expected.heights[index] = column.top;
                    expected.biomes[index] = column.biome;
                    for (dy, state) in column.states.into_iter().enumerate() {
                        expected.states[dy * 256 + index] = state;
                    }
                    expected.postprocessing.extend(
                        column
                            .fluid_postprocessing
                            .into_iter()
                            .map(|y| (index % 16, y, index / 16)),
                    );
                }
                expected
                    .postprocessing
                    .sort_by_key(|&(x, y, z)| (x / 4, z / 4, std::cmp::Reverse(y), x % 4, z % 4));
                assert_eq!(actual, expected, "seed {seed}, chunk {pos:?}");
                if seed == 1 {
                    if let Some(first) = &first_seed_one {
                        assert_eq!(&actual, first, "interleaved world changed seed-one noise");
                    } else {
                        first_seed_one = Some(actual);
                    }
                } else if seed == 2 {
                    assert_ne!(actual.states, first_seed_one.as_ref().unwrap().states);
                }
            }
        });
    }

    #[test]
    fn noise_jobs_clear_every_worker_and_unwind_scope() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let generator = WorldGenerator::new(-7);
        let graph = VanillaGraph::load().unwrap().fork();
        pool.install(|| {
            generator.generate_noise(&mut GeneratedChunk::new(ChunkPos::new(-1, -1)), &graph);
        });
        for (entries, capacity) in pool.broadcast(|_| {
            (
                density::density_cache_entries(),
                density::density_cache_capacity(),
            )
        }) {
            assert_eq!(entries, [0; 5]);
            assert!(capacity <= 5 * (1 << 18), "{capacity} retained buckets");
        }
        pool.broadcast(|_| {
            let result = std::panic::catch_unwind(|| {
                let _scope = DensityScope::new();
                let ctx = generator.chunk_context(ChunkPos::new(0, 0));
                density::evaluate(&graph.final_density, 1.0, 40.0, 1.0, &ctx);
                assert!(density::density_cache_entries().into_iter().any(|n| n > 0));
                panic!("test density scope unwind");
            });
            assert!(result.is_err());
            assert_eq!(density::density_cache_entries(), [0; 5]);
        });
    }

    #[test]
    fn shared_scope_preserves_density_bits_and_reduces_corner_evaluation() {
        use std::cell::Cell;
        thread_local! {
            static CALLS: Cell<usize> = const { Cell::new(0) };
        }
        fn counted_blend(_: i32, _: i32, _: i32, value: f64) -> f64 {
            CALLS.set(CALLS.get() + 1);
            value
        }
        let graph = VanillaGraph::load().unwrap();
        let mut ctx = WorldGenerator::new(1234).chunk_context(ChunkPos::new(-2, 3));
        ctx.blend_density = counted_blend;
        let sample = |per_column: bool| {
            let _scope = DensityScope::new();
            CALLS.set(0);
            let mut bits = Vec::new();
            for index in 0..64 {
                let _column_scope = per_column.then(DensityScope::new);
                for y in MIN_Y..=MAX_Y {
                    bits.push(
                        density::evaluate(
                            &graph.final_density,
                            (-32 + index % 16) as f64,
                            y as f64,
                            (48 + index / 16) as f64,
                            &ctx,
                        )
                        .to_bits(),
                    );
                }
            }
            (bits, CALLS.get())
        };
        let (cold, cold_calls) = sample(true);
        let (shared, shared_calls) = sample(false);
        assert_eq!(cold, shared);
        assert!(shared_calls < cold_calls, "{shared_calls} vs {cold_calls}");
        println!(
            "64-column exact-density probe: {} samples, cold {cold_calls} / shared {shared_calls} blend-density corner evaluations",
            cold.len()
        );
    }

    #[test]
    fn density_scopes_separate_geometry_bounds_and_blender_contexts() {
        use density::{DensityFunction as D, EvalContext, EvaluationMode};
        let offset = D::Cache2d(Box::new(D::BlendOffset(Box::new(D::Y))));
        let noise = D::FlatCache(Box::new(D::Noise {
            name: "temperature".into(),
            xz: 0.25,
            y: 0.5,
        }));
        let blended = D::BlendDensity(Box::new(D::Square(Box::new(D::Y))));
        let interpolated = D::Interpolated(Box::new(D::Add(Box::new(noise), Box::new(blended))));
        let value = D::Add(Box::new(offset), Box::new(interpolated));
        let graph = D::CacheAllInCell(Box::new(D::CacheOnce(Box::new(value))));
        let contexts = [
            EvalContext::default().with_noise_bounds(0, 0, 4),
            EvalContext {
                seed: -1234,
                cell_width: 8,
                cell_height: 4,
                flat_cache_bounds: Some([-2, -2, 0, 0]),
                blend_offset: |x, z| f64::from(x - z),
                blend_density: |x, y, z, value| value + f64::from(x + y + z) * 0.25,
                ..Default::default()
            },
            EvalContext {
                mode: EvaluationMode::Raw,
                ..Default::default()
            },
            EvalContext::default().with_noise_bounds(0, 0, 4),
        ];
        let points = [[1.0, 3.0, 2.0], [-1.0, -3.0, -2.0], [5.0, 9.0, 7.0]];
        let mut first = None;
        for (index, ctx) in contexts.iter().enumerate() {
            let actual: Vec<_> = {
                let _scope = DensityScope::new();
                points
                    .into_iter()
                    .map(|[x, y, z]| graph.evaluate(x, y, z, ctx).to_bits())
                    .collect()
            };
            assert_eq!(density::density_cache_entries(), [0; 5]);
            let expected: Vec<_> = points
                .into_iter()
                .map(|[x, y, z]| {
                    let _scope = DensityScope::new();
                    graph.evaluate(x, y, z, ctx).to_bits()
                })
                .collect();
            assert_eq!(actual, expected, "context {index}");
            if index == 0 {
                first = Some(actual);
            } else if index == 3 {
                assert_eq!(Some(actual), first);
            } else {
                assert_ne!(Some(&actual), first.as_ref());
            }
        }
    }
}
