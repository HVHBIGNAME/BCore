# BCore / vanilla 26.1 parity — 2026-09-13

**Worldgen parity is incomplete.** These measurements cover one seed and three
16×16 regions, not the whole world or all dimensions.

- Seed: `846692123413862008`; protocol: **775** (intentional development target).
- Regions centred at `(0,0)`, `(1000,0)`, `(-2000,3000)`.
- Full vertical range: **Y=-64..319**, 98,304 block states per region.
- Vanilla: live offline server 26.1 at `127.0.0.1:25571`.
- BCore: freshly built `dump_chunk` release executable, not the running server
  or saved BCore chunks. Vanilla captures are reused across comparisons.
- Reference: `crates/bcore-worldgen/data/parity_26_1.json` contains 768 terrain
  heights and 4,608 quart-cell biomes plus hashes of the source captures.
- Raw measurements and binary/capture hashes: [parity-results.json](parity-results.json).

## Results

| Centre | Terrain heights | 3D biome cells | Exact block states | Lowest-log proxy V / B / intersection |
|---|---:|---:|---:|---:|
| (0,0) | 256/256 | 1536/1536 | 91202/98304 (92.78%) | 12 / 16 / 1 |
| (1000,0) | 256/256 | 1536/1536 | 96606/98304 (98.27%) | 16 / 7 / 1 |
| (-2000,3000) | 256/256 | 1536/1536 | 94865/98304 (96.50%) | 13 / 10 / 4 |

Combined block-state match: **282673/294912 = 95.85%**. Air is included, so this
number must not be presented as the completion percentage of the generator.
State comparison includes properties; biome comparison uses names, since the
transitional BCore registry capture and vanilla 26.1 assign some different IDs.
The lowest-log proxy counts branches and fallen logs too; it is not a tree census.

### Corrections to the previous report

The old grid harness compared X-major vanilla output against Z-major BCore output.
Its reported 16–59% terrain match was a transposition error. Coordinate-addressed
comparison shows **768/768 height matches already in the baseline**. This release
does not claim to have repaired that terrain.

The previous report labelled Y=40..120 while actually sampling 40..220. The new
snapshot records explicit bounds, complete coordinate coverage and executable /
capture hashes. Empty biome names from Mineflayer are resolved using the live
configuration registry, rather than treated as valid biome measurements.

Baseline exact-state counts were 91653, 96153, 94766. The current tree changes
improve two regions and worsen `(0,0)`; total improvement is only 101 states.
This is evidence of remaining tree-shape and inter-chunk dependencies, not parity.

## Confirmed fixes

- Parse scalar and range climate values structurally. Scalar depth no longer
  reads the following erosion array. Preserve every biome's registry identity.
- Quantize climate in float before integer squared-distance comparison; include
  squared offset. Store and encode all 1,536 three-dimensional biome cells/chunk.
- Bundle the extracted noise, density, settings and biome data. Unit tests and
  distributed binaries no longer silently fall back to the prototype generator
  when `target/datapack` is absent.
- Restore default forest/birch/plains decorator configurations. Place actual
  leaf-litter states, sampling the provider only after placement predicates pass.
- Stop tree writes wrapping into the opposite edge of the same chunk.
- Sample cached NormalNoise by reference instead of cloning its permutation
  arrays for every sample. Clear density caches on Rayon workers as well as the
  caller. Local warm measurements moved from ~3.1–3.3 s/chunk to ~2.8–2.9 s/chunk;
  this is one-machine evidence, not a benchmark against Java.

## Remaining work, in order

1. A real staged generation region for cross-chunk tree/ore reads and writes.
   Trees currently stop at chunk bounds. The documented ChunkPyramid is not a
   functioning dependency scheduler.
2. Complete tree minimum-size checks, fancy/fallen/mega tree shapes, selectors,
   heightmaps, beehives and leaf-distance propagation. Some variants still use
   approximate substitutes. Ground decoration currently assumes a single base.
3. Vanilla biome zoom and R-tree equal-distance tie behavior. Matching these
   4,608 cells does not establish boundary parity on other seeds.
4. Cave/aquifer/ore parity, biome-specific surfaces, then structures (including
   ancient cities, explaining some deep-dark differences at the origin).
5. Additional seeds, ocean/mountain/biome-boundary samples, Nether and End.
6. Optimize measured hot paths after each exactness check; current performance
   is still far from the project goal.

The river hypothesis at `(1000,0)` was disproved: surface cells are forest; the
full-height sample contains forest and deep_dark, with no river.

## Reproduce

```sh
cargo build --release -p bcore-worldgen --example dump_chunk
# Requires vanilla 26.1 on port 25571 and the existing opped bot account:
python scripts/worldgen_snapshot.py --capture --output target/parity-current.json
# Iterate against the same saved vanilla data:
python scripts/worldgen_snapshot.py --output target/parity-current.json
# Offline, bundled regression test (no server/datapack directory required):
cargo test --release -p bcore-worldgen --test vanilla_reference
```

`scripts/bundle_worldgen.py` regenerates the embedded bundle from existing extracted
data and the BCore configuration capture. `BCORE_DATAPACK` is an explicit override;
missing override files are errors. Old saved worlds retain old chunks: compare
fresh generator output or use a separate new world directory for gameplay tests.
