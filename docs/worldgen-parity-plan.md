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
- For hives, retain occupant order and signed ages. Compare raw tick requests and
  prepared queues at explicitly matched stages; native proto preparation changes
  delays and duplicate handling. Persistence alone does not establish execution.
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
| Biomes | 4,608 captured quart cells; 88,434 native biome-zoom selections; resource-key bridge with 13,932 memberships; native section-Y then X/Y/Z fill verified in 12 chunk-history cases | Broader R-tree tie, worker and saved-history coverage |
| Feature order | All 11 native steps from 54 possible overworld biomes | Complete the implementations at those indices and native inter-chunk scheduling |
| Feature dependencies | Both native pyramids; persistent `GenerationWorld` holders, source guards and once-only feature work; 10 generation tests plus queued adjacent-world integration | Persisted holder/loading integration, bounded eviction and native multi-task history matching |
| Terrain / aquifers | 768 captured heights; 32,256 native aquifer substance/update checks and 16 center samples | Complete density/surface/carver stage comparisons, fluid-tick integration and wider aquifer coverage |
| Carvers / surface resolver | 1,431 independently verified cases: prior 498 + 899 evaluations + 26 surface-voxel cases + 8 palettes; all 54 overworld source biomes, 451 production-adapter evaluations and 13 new native-aquifer voxel cases; 13 tests passed in 0.25 s | Custom heights, retrogen, nonempty blending, fluid ticks and complete mixed-feature regions |
| Ores | 392 native configured cases; 360 complete placement streams for 30 overworld ore placements; real neighbour reads/writes | Full stage/source-chunk ordering; preceding structures/features and varied native regions |
| Standing trees | 438 full native cases (378 configured + 60 placed), 13 variants and 8 selectors; both RNG backends, flags, leaf/edge updates, bee data and tick requests; now called by the live source driver | Native multi-source history matching; remaining variants and contextual edge effects |
| Additional configured trees | 688 native cases / 24 variants on both RNG backends; mangrove roots, ordered writes, effects and RNG; 4 native tree-bin traces / 1,152 operations; fresh fixture/catalog verification | Natural pale-oak callbacks, huge mushrooms, contextual edge effects and identical-hash Java identity ties |
| Cave support shapes | Glow-lichen attachment reuses proven multiface masks: 65 cache-null states / 195 native observations; 10 cave and 6 sculk tests passed | 114 contextual states remain unsupported; broader native mixed-feature regions |
| Density-cache lifetime | Fixed graph/context per Rayon job; 4 terrain regressions, 24,576 exact density samples, cold-column equality and worker/unwind cleanup; measured 7×7 workload 127.61→44.56 s with unchanged result hashes (baseline contended) | Controlled wider scaling/peak-memory measurements and complete stage/source history comparisons |
| Generated-effect persistence | Shared in-memory ownership and `.bcc` v5 (reads v1–v5); pending typed NBT, lazy materialization, native hive data, sculk across 106 states, duplicate marks/ticks | Ticket-driven FULL boundary; persisting holders and unreturned neighbours; gameplay |
| Tick containers | 22 native traces, 4 preparations, 24 conversions, 81 comparator pairs, 12 filters and 11 priority cases; 8 Rust tests | Production handoff from raw requests, world clock/sub-order allocation, cross-chunk dispatch and block/fluid callbacks |
| Fallen trees | 655 native cases through live FeatureRegion (645 writable + 10 fault-injected), original ten preserved; both RNG backends, owning-chunk marks/flags and 89,619 UP/FULL queries; live driver hookup | Native multi-source integration parity and tick execution |
| Historical vegetation fixtures | 37 isolated standing-tree cases and 504 bounded placement-driver cases preserved alongside the 655 fallen cases | These older scopes do not establish full native source scheduling |
| Structure candidates | 54 native grid cases, all frequency reducers | Exclusion zones, biome/terrain admission, weighted entries, concentric rings |
| Mineshafts | 36 layouts / 5,714 pieces; verified 618 postProcess cases, 136 admission cases, two overlapping solid-terrain regions and 144 shape checks; generated loot/spawners retained in `.bcc` v4 | Complete mixed-feature regions and native dependency scheduling |
| Generated entities / queue | 27 native packet samples; shared IDs/UUIDv8 and delivery-state adoption; world-specific Arc-owned jobs with per-world in-flight/RAII cleanup; 234 protocol tests passed in the complete workspace run | Minecart motion, loot/inventory interaction and persistent mutable-entity state |
| Monster rooms | 144 native room/RNG/NBT cases; region placement; client update NBT and backwards-compatible `.bcc` v4 persistence | Full native-region location/order comparisons; preceding structures/features; gameplay/tick behavior |
| Other structures | Runtime villages/cities/trail ruins/trial chambers, terrain adaptation and typed template data; 672 alias resolutions; scattered treasure/hut/temple starts, references, clipping and 107 native piece-storage snapshots | Remaining families, village/hut entity factories/finalization, wider mixed-feature histories and unsupported processors |
| Lighting | Real INITIALIZE_LIGHT/LIGHT and ordered later-source updates; 18 native light tests, 19 packet cases, lazy/materialized/absent storage and BCC round trips | Broader mixed-feature histories, FULL/loading lifecycle and contextual callbacks |
| Structure/feature handoffs | Native decorated-pot handoff fixed; 58 stair types / 10,212 updates and 9,360 bamboo updates independently repeated, including read/tick ordering | Remaining contextual block callbacks and broader cross-source coverage |
| Fossils | 16 native templates, 76 component cases and real cross-chunk feature/FULL histories; shared RNG, shape callbacks and retained WG maps | Wider terrain/history coverage and remaining configured features |
| Desert pyramids | 62 admissions, 137 native placement passes, 98 reference comparisons; geometry, cellar, archaeology and BCC persistence | FULL/ticking lifecycle and wider structure mixtures |
| Natural mob generation | Retained SPAWN with independent entity entropy/region RNG; 456 native proto-save→LOAD→pairing cases across 19 CREATURE types, BCC and queued delivery | Live server clock/settings hookup, generic structure entities and ticking simulation |
| Nether / End | No full-generation parity evidence | Dedicated pipelines, features, structures and fixtures |

The **2026-10-02 shared-context snapshot** records **92,242 / 96,618 / 97,187**
matching states: **286,047 / 294,912 = 96.99% including air**, 600 fewer matches
than the historical checkpoint. All 768 heights and 4,608 quart biomes match.
Its eight requested chunks have finished supported incoming work, but none has
complete native coverage; 138 placed streams remain missing across 40 sources.
The [parity report](parity-report.md) retains binary/capture provenance, source
coverage and the historical **97.20%** result. The numerical-phase **123 / 2**
worldgen and pre-streaming **178-pass** protocol totals are historical checkpoints.

The previous complete release verification passed **628 tests, with 0 failures and
4 ignored** (371 worldgen / 248 protocol). The final frozen binary also matched
blocks, biomes, source order and scored light in **69 requests across 11 native
histories**. See the current [native-history report](parity-report.md#native-history-integration--2026-10-04)
for that checkpoint. The [October 7 milestone](generation-milestone-2026-10-07.md)
records the subsequent fossil/pyramid/deferred-NBT repairs and combined validation.

The earlier **2026-10-02 workspace release run passed 514 tests, with 0 failures
and 4 ignored**, including **271 worldgen / 234 protocol** passes. It completed
the 7×7 persistence and 64-chunk terrain/fluid checks. The ignored tests, timings,
command and saved logs are recorded in the parity report's regression section.

The earlier live integration passed **7 base-feature tests, 10 generation tests and 17
targeted protocol tests**, including actual generated sculk/marks, all 106 native
sculk-compatible states, and shared queued generation across world clones. Current
generation at that checkpoint executed lighting; SPAWN and FULL remained pending. The [October 4 native
history report](parity-report.md#native-history-integration--2026-10-04) records newer
matched-history results and the remaining deferred-NBT/WG lifetime differences.

The **2026-09-29** documentation follow-up verified the graph, hive and tick
fixtures and hive rewrite traces with independent native recaptures. **28 region tests, 8 tick-container
tests, 18 storage tests and 21 protocol integration tests passed**; the breakdown
is in [tree-effects.md](tree-effects.md#verification-and-reproduction).
The earlier full worldgen log, `target/worldgen-tree-effects-final.log`, records
**182 passed / 2 ignored**, following the earlier **163 / 2** checkpoint.
That checkpoint had no completed post-change protocol suite. Historical carver,
tree, queue and world test results remain in [parity-report.md](parity-report.md).
Mth timings in [numeric-parity.md](numeric-parity.md) remain per-call measurements,
separate from the historical ~1.8× erosion-noise result.

See [entity-streaming.md](entity-streaming.md) for the immutable generated-data
identity contract, ownership, verified packet coverage and same-connection
`ChunkDeliveryState` adoption contract. The opaque non-Clone token consumes the
previous view's delivery state, retaining exact entity ownership and teleport IDs;
a destination with existing delivery/teleport state returns it intact on rejection.

Production carvers use the real surface resolver and required
`crates/bcore-worldgen/data/surface_climates_26_1.json`; the old scripted voxel cases
retain their narrower scope. Queue routing now retains the requesting world.
The source driver now uses a world-scoped `FeatureRegion`, native decoration indices
and shared RNG state. Server `World` and the multi-coordinate diagnostic retain the
same context across requests. Owning-chunk data survives `.bcc` v5, including sculk
NBT and unconsumed marks. Persisted holder/loading integration, native schedule
traces, remaining features/structures and runtime tick execution are still pending.

## Implementation order

1. **Numerical oracle first.** Iterate with `math_lab` on identical Java/Rust
   inputs, inspect the first differing bits, then benchmark verified changes.
   [numeric-parity.md](numeric-parity.md) contains the commands, request format
   and editable formula experiments. Use chunk captures as integration checks.
2. **Stage oracle and persistent generation history.** Both graphs, live holders,
   stage claims and source-relative guards are implemented. Add persisted status,
   saved-neighbour hydration and safe eviction. Capture actual multi-task execution
   and distinguish generation from loading before asserting native source order.
   Compare multi-chunk output under matched request histories and worker counts;
   preserve any native history dependence rather than assuming a global X/Z order.
   [feature-scheduling.md](feature-scheduling.md) gives the exact radii and sequence.
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
    region and shared source driver. Complete the missing source operations and
    validate native scheduling histories. Connect tick-container helpers to the
    runtime, then extend natural pale-oak callbacks and contextual effects.
6. **Coverage and optimization.** Expand seeds, biome classes, dimensions and
   boundary fixtures. Profile validated stages and retain exact outputs while
   optimizing; do not substitute approximate algorithms for speed.

Measurements and reproduction commands: [parity-report.md](parity-report.md).
