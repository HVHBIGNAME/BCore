# Vanilla generation parity

Target: Minecraft Java **26.1**, protocol **775**, the same seed, generator
settings and datapack. Completion requires exact generated content: biomes,
terrain, caves, aquifers, ores, vegetation and structures, including mineshafts
and monster rooms (dungeons). Nether and End require their own coverage.

## Acceptance

- Compare canonical block names **and all state properties**, quart biomes,
  heightmaps, structure starts/references/piece bounds and generated block entities.
  Numeric registry IDs alone are insufficient across differing registry captures.
- For chests and spawners, compare generated NBT, loot table/seed and entity
  configuration at the same generation stage, before loot opening or gameplay ticks.
- Include generated entities' type, initial position and loot data. Validate their
  packet bytes and chunk-view lifecycle separately. BCore reconstructs UUIDv8
  identities from immutable generated records; native random UUID equality and
  persistent mutable gameplay-entity identity are outside that contract.
- Capture native stage outputs before player edits, fluid/leaf ticks or mob activity.
  Retain source JAR, probe, configuration and capture hashes.
- Exercise multiple positive/negative/extreme seeds, negative coordinates, region
  and biome boundaries, ocean/mountain/cave biomes and structures crossing chunks.
- Compare isolated features with exact RNG continuation **and** complete regions.
  Generate those regions in multiple request orders and worker counts.
- A passing sample suite is evidence for its stated coverage. Missing features or
  known approximations preclude declaring the generator 100% compatible. A global
  match percentage dominated by air is not a completion percentage.

## Current coverage

| Area | Evidence | Missing work |
|---|---|---|
| Numerical kernels | 62,649 native bit-exact values / 895 cases; f32/f64 math, all 62 NormalNoise parameter sets, raw router and NoiseChunk densities; reversed inputs/four workers | Expand targeted boundary/configuration coverage and use this as the first debugging loop |
| Shared Mth lookup | 65,536 native table entries; 1,903 angle cases / 3,806 outputs with zero bit mismatches; full-table hash verified; three tests including eight-thread sharing; 256 KiB OnceLock table integrated into carver and ore-radius calls | Whole-chunk performance and final integrated regression measurements |
| RNG | Native double-stream hashes and continuation; existing integer tests | Additional legacy paths and feature-specific stream coverage |
| Biomes | 4,608 captured quart cells; 88,434 native biome-zoom selections; region-aware tree admission | Global feature admission, R-tree ties and full multi-seed biome coverage; compare native/BCore registries by name |
| Feature order | All 11 native steps from 54 possible overworld biomes | Complete the implementations at those indices and native inter-chunk scheduling |
| Terrain / aquifers | 768 captured heights; 32,256 native aquifer substance/update checks and 16 center samples | Complete density/surface/carver stage comparisons, fluid-tick integration and wider aquifer coverage |
| Carvers / surface resolver | 1,431 independently verified cases: prior 498 + 899 evaluations + 26 surface-voxel cases + 8 palettes; all 54 overworld source biomes, 451 production-adapter evaluations and 13 new native-aquifer voxel cases; 13 tests passed in 0.25 s | Custom heights, retrogen, nonempty blending, fluid ticks and complete mixed-feature regions |
| Ores | 392 native configured cases; 360 complete placement streams for 30 overworld ore placements; real neighbour reads/writes | Full stage/source-chunk ordering; preceding structures/features and varied native regions |
| Standing trees | 438 full native cases (378 configured + 60 placed), 13 variants and 8 selectors; both RNG backends, flags, leaf/edge updates, bee data and tick requests; 29,873-state table; 127 fixed-plan target replays including 63 incoming-only | Native full-source scheduler/global region hookup; TreeEffects transfer/persistence; jungle and contextual edge effects |
| Fallen trees | 655 native cases through live FeatureRegion (645 writable + 10 fault-injected), original ten preserved; both RNG backends, owning-chunk marks/flags and 89,619 UP/FULL queries | Global region-vegetation hookup and tick execution |
| Historical vegetation fixtures | 37 isolated standing-tree cases and 504 bounded placement-driver cases preserved alongside the 655 fallen cases | These older scopes do not establish full native source scheduling |
| Structure candidates | 54 native grid cases, all frequency reducers | Exclusion zones, biome/terrain admission, weighted entries, concentric rings |
| Mineshafts | 36 layouts / 5,714 pieces; freshly verified 618 postProcess cases, 136 admission cases, two overlapping solid-terrain regions and 144 shape checks; generated loot/spawners and v3 persistence | Complete mixed-feature regions and native dependency scheduling |
| Generated entities / queue | 27 native packet samples; shared IDs/UUIDv8 and delivery-state adoption; world-specific Arc-owned jobs with per-world in-flight/RAII cleanup; 6 queue and 26 world tests passed | Newest combined suite totals; minecart motion, loot/inventory interaction and persistent mutable-entity state |
| Monster rooms | 144 native room/RNG/NBT cases; region placement; client update NBT and backwards-compatible v3 persistence | Full native-region location/order comparisons; preceding structures/features; gameplay/tick behavior |
| Other structures | Candidate coverage for selected random-spread sets | Pieces/templates/jigsaw, processors, terrain adaptation and generated NBT |
| Nether / End | No full-generation parity evidence | Dedicated pipelines, features, structures and fixtures |

The numerical-phase worldgen result is **123 passed / 2 ignored**; the protocol
baseline **before the latest streaming changes** is **178 passed**. The recorded
three-region snapshot remains **92,834 / 97,031 / 96,782** matching states, or
**97.20% including air**. Historical totals, executable provenance and reproduction
commands are in [parity-report.md](parity-report.md).

Newest confirmed targets: **13 carver tests**, **6 queue tests**, **26 world tests**
and **45 tree tests**: 5 standing-region, 18 tree, 4 decoration, 5 fallen-region,
4 bounded-vegetation-region and 9 fallen-reference. Native capture and independent
verification passed for the 1,431 carver cases and both new standing fixtures.
The newest combined protocol suite is pending; the fresh worldgen suite passed
**163 tests with 2 ignored**, and the fresh snapshot is recorded above. Mth timings in
[numeric-parity.md](numeric-parity.md) remain per-call measurements, separate from
the historical ~1.8× erosion-noise result.

See [entity-streaming.md](entity-streaming.md) for the immutable generated-data
identity contract, ownership, verified packet coverage and same-connection
`ChunkDeliveryState` adoption contract. The opaque non-Clone token consumes the
previous view's delivery state, retaining exact entity ownership and teleport IDs;
a destination with existing delivery/teleport state returns it intact on rejection.

Production carvers use the real surface resolver and required
`crates/bcore-worldgen/data/surface_climates_26_1.json`; the old scripted voxel cases
retain their narrower scope. Queue routing now retains the requesting world.
Region-aware tree kernels are implemented, but **global region vegetation is not
hooked up**: fixed source plans are not the native scheduler. Hive NBT/tick requests
remain in `TreeEffects`, and `finish_chunk` asserts transfer before finalization.
Persistence and tick execution remain pending.

## Implementation order

1. **Numerical oracle first.** Iterate with `math_lab` on identical Java/Rust
   inputs, inspect the first differing bits, then benchmark verified changes.
   [numeric-parity.md](numeric-parity.md) contains the commands, request format
   and editable formula experiments. Use chunk captures as integration checks.
2. **Stage oracle and shared generation region.** Split terrain, carvers and
   features into real stages. Read/write neighbours at the required status;
   reproduce stage and source-chunk ordering. Replace the no-op ChunkPyramid
   barriers and the ore pass's bounded 3×3 source replay. Neighbour heights now
   read real chunks, backed by a bounded immutable pre-feature cache.
   Verify order-independent final output against native multi-chunk captures.
3. **Underground generation.** Compare densities, carvers and aquifers before
   features; then native ore shapes, targets, air-exposure draws and placement
   streams. Missing cave biomes/features cannot be compensated by tuning ore rates.
   Carver reach, seed/start decisions, thickness/width RNG, geometry traces,
   full-state replaceability and bounded voxel rasterization/masks now have native
   coverage. The real surface resolver is verified for the overworld baseline;
   extend custom-height/retrogen/blending coverage and complete native region/tick
   comparisons.
4. **Mineshafts and monster rooms.** Build native piece/room fixtures first.
   Implement geometry and RNG, then shared-region writes and generated NBT.
   Monster rooms use placed features, not random-spread structure placement.
   MonsterRoomFeature and generated chest/spawner data are implemented and
   independently tested. Mineshaft admission, piece post-processing, loot
   entities/spawners, references and bounded region replay have native fixtures.
   Static generated minecarts now stream with shared identities and chunk-view
   lifecycle handling, including verified delivery-state adoption and the native
   minecart pairing packet and world-specific queue. Complete mixed-feature
   native-region parity and run the final combined checks;
   gameplay, motion, loot opening/inventory interaction and spawner simulation
   remain separate implementation work.
5. **Remaining biome features and structures.** Complete selectors/decorators,
   surfaces, biome zoom, structure templates/jigsaw and terrain adaptation.
   Standing/fallen trees and eight placed selectors now work through the live
   region. Complete the native source scheduler, global hookup and `TreeEffects`
   transfer/persistence, then extend explicitly unsupported jungle/contextual
   effects.
6. **Coverage and optimization.** Expand seeds, biome classes, dimensions and
   boundary fixtures. Profile validated stages and retain exact outputs while
   optimizing; do not substitute approximate algorithms for speed.

Measurements and reproduction commands: [parity-report.md](parity-report.md).
