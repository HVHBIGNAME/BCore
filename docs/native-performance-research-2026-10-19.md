# Native performance research — 2026-10-19

**Strongest additional candidates:** indexed per-node density caches, seed-bound noise samplers, dimension-aware batching, and reuse of Perlin lattice work along columns. These address concrete work still present in BCore; rankings express applicability, not measured speedup.

Research-only source review for Rust `native26.1`. Acceptance remains exact blocks, heightmaps, and ordered effects, including native floating-point evaluation and stateful query order. Subsequent measurements are recorded in [the performance experiments](performance-experiments-2026-10-20.md).

## Provenance and comparison scope

GitHub trees, commit metadata, and raw source were read with `gh`/`webfetch`. Links below pin the supplied `master` snapshots; dates are GitHub committer dates (UTC).

| Project | Reviewed commit | Date | Scope and license |
| --- | --- | --- | --- |
| Pumpkin | [`f4dcfd45ad010b711fece42040dd465d33901034`](https://github.com/Pumpkin-MC/Pumpkin/commit/f4dcfd45ad010b711fece42040dd465d33901034) | 2026-10-19 | Rust; workspace version [`0.2.0+26.3-26.51`][p-build], not a 26.1 snapshot. Server code is [GPLv3][p-license]. |
| Cuberite | [`7fd3fa5c9345a3f1b949c0988c4849db00a68486`](https://github.com/cuberite/cuberite/commit/7fd3fa5c9345a3f1b949c0988c4849db00a68486) | 2025-12-16 | C++; documents [1.8–1.12.2 protocol support][c-version], with its own terrain generators. [Apache-2.0][c-license]. |
| FerrumC | [`51830e99675deae1f6cdbb62bf5d32b3d25c1c76`](https://github.com/ferrumc-rs/ferrumc/commit/51830e99675deae1f6cdbb62bf5d32b3d25c1c76) | 2026-08-14 | Rust; documents [1.21.8][f-version]. Reviewed generation is custom climate/heightfield terrain, not native26.1. [MIT][f-license]. |

BCore observations refer to the in-progress working tree based on `05329feaec0bdf0553e04d4154c8c5e29bc7f429`, not an immutable clean checkout. Local line references can move as concurrent work lands. This document describes algorithms in original prose; it incorporates no competitor implementation code or data.

## BCore techniques already present

- Graph loading uses `OnceLock`; named density references share immutable `Arc` identity. Seeded `NormalNoise` construction is cached, although sampling still performs registry/seed/name map lookups: [graph loading](../crates/bcore-worldgen/src/lib.rs#L1545-L1566), [references](../crates/bcore-worldgen/src/density.rs#L73-L85), [noise lookup](../crates/bcore-worldgen/src/simplex.rs#L293-L330).
- NOISE already owns one chunk-wide aquifer and ore context; each column uses fixed-size density scratch: [terrain.rs:68–109](../crates/bcore-worldgen/src/generation/terrain.rs#L68-L109), [lib.rs:949–997](../crates/bcore-worldgen/src/lib.rs#L949-L997). Recommending per-chunk context reuse as a new optimization would duplicate this work.
- Interpolation already caches cell corners and shares adjacent vertices. The remaining opportunity is cache access/layout, not discovering corner reuse: [density.rs:638–699](../crates/bcore-worldgen/src/density.rs#L638-L699).
- Range selection is lazy, zero-amplitude octaves are skipped, and blended noise already skips the unneeded limit branch: [range](../crates/bcore-worldgen/src/density.rs#L448-L462), [octaves](../crates/bcore-worldgen/src/noise_perlin.rs#L324-L342), [blended noise](../crates/bcore-worldgen/src/noise_perlin.rs#L468-L493).

## Ranked transferable methods

### 1. Give cache nodes indexed, bounded storage

**Observed:** Pumpkin's [`Cache`][p-cache] owns a last-position/value pair plus an optional density volume and buffer. In-volume reads use an [arithmetic index][p-volume], rather than hashing coordinates. Its [chunk router][p-chunk] keeps mutable chunk cache nodes beside references to immutable prototype components.

**BCore application:** replace eligible regular-lattice lookups in [the five thread-local maps](../crates/bcore-worldgen/src/density.rs#L235-L254) with node-ID-indexed state: a corner grid, the admitted quart-X/Z flat grid, and bounded per-cell values. At BCore's [384-block height](../crates/bcore-worldgen/src/lib.rs#L72-L78) and [4×8×4 cell size](../crates/bcore-worldgen/src/density.rs#L209-L231), one full-chunk corner lattice is `5×49×5 = 1,225` values per interpolator/mode; one block cell is 128 values. These are geometric storage counts, not benchmark results.

Retain complete scope identity and validity: seed, graph, evaluation mode, bounds/blender context, and cell/point coordinates. Preserve generic/raw and out-of-grid behavior. Pumpkin's codegen [maps several legacy cache markers to one `Cache` kind][p-markers]; that newer semantic model must not replace BCore's distinct 26.1 marker behavior.

### 2. Bind samplers once; lower graphs to indexed components and invariant-axis batches

**Observed:** Pumpkin [constructs seeded samplers while building a contiguous prototype stack][p-proto], then evaluates children by [component indices][p-stack]. Its code generator [propagates coordinate dependencies][p-axes] and [inserts slices for invariant axes][p-slice-insertion]; runtime [`Slice::sample_volume`][p-slice] evaluates a reduced-dimensional volume and broadcasts it. FerrumC also [constructs decorators/noise state once][f-context] and [reuses per-column climate/height arrays][f-reuse].

**BCore application:** resolve noise names to seed-bound sampler handles during graph preparation, removing repeated string/seed-map lookups from `NoiseRegistry::sample`. An indexed plan can also own cache slots and 1D/2D/3D dependency metadata. Compute truly X/Z-only functions once per required X/Z sample, and Y-only functions once per required Y. Existing `Cache2d` helps marked nodes, but does not itself provide general dependency analysis or direct sampler binding.

Dependencies must include shifted coordinates, nested splines, evaluation modes, and blender hooks. Preserve native spline `f32` boundaries and density `f64` operations; do not merge nodes merely because their mathematical formulas resemble each other.

**Compiled-plan limit:** Pumpkin's tool [emits generic direct evaluation functions][p-emitter], but the inspected `ChunkNoiseRouter` path uses the indexed enum stack. Emission is not evidence that generated functions accelerate production generation. Its [emitted binary expressions][p-emitted-binary] are eager even where the scalar stack evaluator short-circuits. Carry native laziness into any BCore lowering.

### 3. Batch noise by column/lattice cell and reuse invariant work

**Observed:** Pumpkin's [`PerlinNoise::add_to_volume`][p-perlin] hoists X/Z wrapping, floors, fade values, and X permutations outside the Y loop; corner data is refreshed only when the integer Y cell changes. Cuberite [groups samples sharing integer coordinates][c-grouping] and [reuses overlapping lattice corners with two workspaces][c-corners]. Its selectable/default [BiomalNoise3D path][c-selection] [fills coarse arrays, reuses a workspace, and upscales][c-generation].

**BCore application:** a column/batch interface below `PerlinNoise::value` can reuse exact X/Z preparation and gradient/permutation identities across consecutive Y samples. Today [each scalar ImprovedNoise call](../crates/bcore-worldgen/src/noise_perlin.rs#L232-L283) reconstructs that work. Keep each lane's octave accumulation and multiplication order unchanged.

Transfer the reuse pattern, not competitor arithmetic: Pumpkin's [X/Z partial dot products][p-partial-dot] and [incremental cell interpolation][p-interpolation] use different grouping/precision; Cuberite uses [`float` noise][c-noise-type] and [incrementally advances sample coordinates][c-grouping]. BCore explicitly distinguishes X/Y/Z interpolation from material Y/X/Z interpolation. Buffer layout may change without changing biome-query, aquifer/material, or effect-commit order.

### 4. Propagate conservative bounds and keep batch branches lazy

**Observed:** Pumpkin [computes node ranges during prototype construction][p-bounds]. Its [scalar `Min`/`Max` evaluator][p-binary] skips RHS when LHS is strictly below RHS minimum or above RHS maximum. However, its [binary volume evaluator][p-eager-volume] computes both inputs, and [volume range choice][p-eager-range] computes both branches.

**BCore application:** add trustworthy 26.1 bounds to indexed nodes and use native comparison rules to avoid provably unnecessary subtrees. Current [Min/Max](../crates/bcore-worldgen/src/density.rs#L429-L432) evaluate both children. For batches, preserve conditional work through masks or homogeneous-region selection; eagerly filling every branch can erase the gain.

Bounds must cover actual floating-point results and exceptional values, not estimated noise deviation or sampled extrema. Unknown bounds disable pruning. Pumpkin's constants and `Lerp` endpoint shortcuts are not authority for changing BCore's native arithmetic.

### 5. Specialize proven coordinate cases

**Observed:** Pumpkin's [`wrap`][p-wrap] returns its input inside a guarded no-wrap interval, with the original floor-based formula outside it. The guard is deliberately below half the period. BCore's [wrap](../crates/bcore-worldgen/src/noise_perlin.rs#L497-L500) always evaluates the floor expression. FerrumC uses [masks for section-local coordinates][f-local-coordinates] and [shifts for packed section indices][f-packing].

**BCore application:** the guarded wrap path is a small, concrete candidate. Preserve the floating-point boundary near half-period; a naive mathematical interval can differ when the addition rounds upward. For proven integral block coordinates and 4/8 cell dimensions, a specialized cell locator can use signed shifts/masks. Keep arbitrary floating coordinates on their existing path. BCore already masks Perlin permutations and shifts quart coordinates, so those individual operations are not new opportunities. No SIMD instruction speedup is implied by these source patterns.

## Reuse, SIMD, allocators, and build settings

- **Scratch reuse:** Cuberite's [`OctavedNoise::Generate3D`][c-workspace] accepts caller-owned workspace and reuses it across octaves. Pumpkin's [`DensityBuffer` pool][p-pool] rounds capacity to 16 elements, searches for a suitable buffer up to twice the requested capacity, and returns buffers on drop, capped at 1,024 buffers. That is a count cap, not an aggregate byte cap. Reusable, byte-bounded `f64` scratch is relevant if BCore adds batches; its existing fixed column scratch already removes that allocation.
- **Material batching:** Pumpkin [precomputes final-density and optional vein arrays][p-material-batch], then supplies values to material sampling. The transferable part is pure-value reuse. BCore's native material interpolation phase and ordered effects still govern consumption; FerrumC's [cached interior-column reuse][f-reuse] similarly does not authorize replacing BCore's stateful biome/feature traversal with position-only generation.
- **SIMD evidence:** the inspected Pumpkin Perlin loop is scalar Rust. FerrumC's [terrain sampler][f-noise] explicitly uses `noise::Fbm<Perlin>` instead of the other branch's `simdnoise` fork. Its inspected [AVX2 routine][f-simd] performs big-endian byte conversion with runtime feature detection and a scalar fallback. This supports an integer/serialization SIMD pattern, not a claim of SIMD native-density generation or bit-exact vector noise.
- **Build settings:** Pumpkin [release][p-build] enables LTO and one codegen unit, with overrides of 16 for large libraries including `pumpkin-world`. FerrumC's [`production`/`hyper` profiles][f-build] enable LTO and one codegen unit; ordinary `release` has no such explicit override. `hyper` explicitly selects `panic = "abort"` and `overflow-checks = false`. Cuberite [enables supported IPO][c-lto] and [CPU-native flags plus non-MSVC `-ffast-math`][c-flags]. LTO/codegen settings are candidates, not demonstrated gains; fast-math is unsuitable for native26.1 exactness. BCore's [workspace manifest](../Cargo.toml#L13-L22) currently specifies package-level dev optimization, with no explicit release profile there.
- **Allocators:** FerrumC's [explicit allocator selections][f-allocator] are optional `dhat` instrumentation or Tracy wrapping `System`; they do not demonstrate a production allocator speedup. The source-backed memory lever here is reduced allocation and scratch reuse, not a justified mimalloc/jemalloc recommendation.

## Limits on performance comparisons

Cuberite's coarse `5×5×33` sampling feeds its own 256-high terrain formulas. FerrumC's [caves][f-caves] use a 2×6×2 coarse grid over a restricted Y range, and its [biome output][f-biomes] chooses one dominant biome for all sections. Neither is equivalent work to BCore's exact native26.1 pipeline. Pumpkin's inspected density/volume path uses `f32` and different interpolation arithmetic despite its vanilla-oriented goals.

[Pumpkin's performance goals][p-goals], [Cuberite's compilation-speed estimate][c-marketing], and [FerrumC's loading demonstrations][f-marketing] establish no hardware-, version-, or workload-matched speed ratio. No builds, profiling, or benchmarks were performed for this report. Source presence establishes the techniques above; actual BCore gains require matched measurements.

[p-build]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/Cargo.toml#L117-L137
[p-license]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/README.md#L118-L122
[p-cache]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/chunk_density_function.rs#L33-L113
[p-volume]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-util/src/noise/volume.rs#L67-L160
[p-chunk]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/chunk_noise_router.rs#L219-L285
[p-markers]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/tools/pumpkin-codegen/src/noise_router.rs#L2759-L2786
[p-proto]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/proto_noise_router.rs#L345-L422
[p-stack]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/chunk_noise_router.rs#L122-L178
[p-axes]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/tools/pumpkin-codegen/src/noise_router.rs#L648-L708
[p-slice-insertion]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/tools/pumpkin-codegen/src/noise_router.rs#L815-L846
[p-slice]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/density_function/misc.rs#L428-L533
[p-emitter]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/tools/pumpkin-codegen/src/noise_router.rs#L2146-L2191
[p-emitted-binary]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/tools/pumpkin-codegen/src/noise_router.rs#L1262-L1295
[p-perlin]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-util/src/noise/perlin.rs#L296-L342
[p-partial-dot]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-util/src/noise/perlin.rs#L51-L59
[p-interpolation]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/chunk_density_function.rs#L225-L263
[p-bounds]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/proto_noise_router.rs#L533-L581
[p-binary]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/density_function/math.rs#L130-L187
[p-eager-volume]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/density_function/math.rs#L216-L255
[p-eager-range]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/density_function/misc.rs#L602-L630
[p-wrap]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-util/src/noise/perlin.rs#L28-L69
[p-pool]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/router/density_volume.rs#L6-L91
[p-material-batch]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/crates/pumpkin-world/src/generation/noise/mod.rs#L189-L217
[p-goals]: https://github.com/Pumpkin-MC/Pumpkin/blob/f4dcfd45ad010b711fece42040dd465d33901034/README.md#L19-L28
[c-version]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/README.md#L6-L10
[c-license]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/README.md#L52-L56
[c-grouping]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/src/Noise/InterpolNoise.h#L470-L511
[c-corners]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/src/Noise/InterpolNoise.h#L235-L276
[c-selection]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/src/Generating/ShapeGen.cpp#L78-L129
[c-generation]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/src/Generating/Noise3DGenerator.cpp#L585-L642
[c-noise-type]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/src/Noise/Noise.h#L8-L9
[c-workspace]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/src/Noise/OctavedNoise.h#L103-L161
[c-flags]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/SetFlags.cmake#L104-L149
[c-lto]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/CMakeLists.txt#L95-L104
[c-marketing]: https://github.com/cuberite/cuberite/blob/7fd3fa5c9345a3f1b949c0988c4849db00a68486/README.md#L29-L34
[f-version]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/README.md#L29-L33
[f-license]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/LICENSE#L1-L9
[f-context]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/lib/world_gen/src/lib.rs#L101-L154
[f-reuse]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/lib/world_gen/src/lib.rs#L359-L375
[f-noise]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/lib/world_gen/src/terrain_noise.rs#L3-L95
[f-caves]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/lib/world_gen/src/caves.rs#L16-L83
[f-biomes]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/lib/world_gen/src/lib.rs#L397-L415
[f-local-coordinates]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/lib/world/src/pos.rs#L53-L59
[f-packing]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/lib/world/src/pos.rs#L335-L355
[f-simd]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/lib/utils/general_purpose/src/simd/arrays.rs#L80-L137
[f-build]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/Cargo.toml#L89-L108
[f-allocator]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/src/bin/src/main.rs#L22-L29
[f-marketing]: https://github.com/ferrumc-rs/ferrumc/blob/51830e99675deae1f6cdbb62bf5d32b3d25c1c76/README.md#L50-L83
