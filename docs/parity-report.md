# BCore / vanilla 26.1 parity

**Worldgen parity is incomplete.** Component checks and the whole-chunk snapshot
below have different coverage; neither establishes all-world/all-dimension parity.

Latest full workspace verification: **662 passed, 0 failed, 4 ignored** in
release, including **394 worldgen and 259 protocol** tests. The previous 628-pass
checkpoint and earlier [regression suites](#regression-suites) remain recorded below.

## Terrain and lifecycle repairs — 2026-10-08

Three parallel workers implemented native fossils, desert pyramids and pending
block-entity materialization. Their immutable before/after histories reproduce
**269 + 103 fossil** and **7,627 desert-pyramid** differing block observations.
See [the milestone report](generation-milestone-2026-10-07.md) for witnesses,
component coverage, RNG contracts and the remaining implicit-FULL differences.

After integration, the generation suite passed **57/57** and focused protocol
storage/delivery passed **12/12**, including the pyramid↔pending-BE boundary and
456 native mob LOAD/pairing records. The final workspace run passed **662 tests**
in 60 target summaries. An earlier run's one standing-tree expectation omitted the
empty hive materialized by an explicit native lookup; its lifecycle setup was
corrected while preserving the native block, RNG and occupant comparisons.

One frozen replay executable then verified **88 requests across 14 histories**:

| Scored output | Observations | Differences |
|---|---:|---:|
| Block states | 8,650,752 | 0 |
| Quart-biome cells | 135,168 | 0 |
| FEATURES source order | 88 request snapshots | 0 |
| Serialized light | 40 scored snapshots | 0 |
| Logical structures and ordered postprocessing | All scored snapshots | 0 |
| Block-entity payloads | Logical fields | 36 |
| WG heightmap presence | Both maps across repeated snapshots | 70 |

The new fossil histories improve **269 → 0** and **103 → 0** differing state
observations; the desert-pyramid history improves **7,627 → 0**. Before/after
comparisons use the identical native captures. All 88 requests still report
incomplete native coverage. The 36 remaining BE fields belong to the final LIGHT
read after Vanilla's implicit ticket-driven FULL transition; WG maps also remain
until real FULL conversion is implemented.

[Published checkpoint](metrics/checkpoint-2026-10-08.json) contains per-request
counts and artifact hashes. Local evidence:

- `target/generation-checks-20261001/tests-1791426197736092900.{json,log}`
- `target/full-parity-20261003/integrated-histories-1791426197736092900/`
- `target/full-parity-20261003/combined-input-audit-20261008-final.json`

The audit verifies archive contents, binary/result hashes, native provenance and
current-source equality. The previous failed test run and older frozen replays
remain preserved. This checkpoint expands the earlier 69-request corpus; it does
not replace the separate historical 96.99% region measurement.

The current `.bcc` writer is v5 (reads v1–v5). Pending typed NBT is separate from
materialized entities; a FULL-request BE materialization boundary does not claim
FULL conversion. SPAWN executes with explicit independent world inputs; live
server clock/settings hookup, FULL/ticket transitions and ticking remain incomplete.

## Native-history integration — 2026-10-04

The actual-server oracle now compares matching bootstrap/request histories, with
frozen BCore source/binary hashes and strict native terminal/stage/blob validation.
The historical region percentage below remains tied to its older executable.

- Trial-chamber generation was stopping at the decorated-pot update-tag handoff,
  so a neighbouring source never reached its andesite placement. Native load/full/
  update capture covers **80 cases**, including every trial template pot payload,
  all 16 block states and malformed/default codec boundaries. Independent JVM
  repetition matched. The fix preserves ordered sherds, partial list decoding,
  loot-vs-item semantics, numeric widths and custom-only client update tags.
- The corrected **8-request trial history** matches blocks, quart biomes, source
  order and scored light. All nine dependency chunks also match immediately before
  and after the previously missing andesite feature, including both RNG witnesses
  and WG heightmaps. **10 structure-runtime tests** and **12 protocol structure/
  sculk/NBT tests** passed; the latter includes all 80 pot round trips.
- The pot-fix binary completed **49 requests in nine expanded histories**: origin,
  distant regions, seeds 0/42, bootstrap, adjacent/reversed requests, trail ruins
  and LIGHT-prefix requests. Block states, biomes, source order and scored light
  have zero differences. Native deferred `DUMMY` sculk tags remain distinct from
  BCore's eager saved defaults before native materialization: **16 field differences
  in the origin FEATURES snapshot and 80 across LIGHT-prefix snapshots**. Those
  differences remain visible in the metadata comparator; FULL samples materialize
  the entities and their payloads match.
- Buried treasure, swamp huts and jungle pyramids are connected to retained
  starts/references, native structure-slot order and source-clipped placement.
  The production adapter passes the six real native admission anchors and 216
  reference-membership comparisons. All **107 native piece snapshots** preserve
  mutable flags and cached reference boxes through `.bcc` storage. Witch/cat
  requests are retained explicitly; factory/finalization is still pending.
- A new 12-request actual-server history spans the three scattered families;
  **4805 native blobs** verify with no in-flight stages. Treasure/hut samples match
  blocks, biomes, metadata and scored light. Jungle adjacency exposed tree-edge
  callbacks on cobblestone stairs and bamboo. The native stair corpus has **58
  types / 10212 updates**; bamboo has **9360 updates** and support predicates checked
  over all 29873 states. Both were independently repeated. Their implementations
  preserve native world-read, state-update and tick-request ordering.

The final frozen binary completed **69 requests across 11 histories** (the expanded
57-request wave plus the 12 scattered requests): zero block-state, quart-biome,
source-order or scored-light differences. Structure starts/references and ordered
postprocessing also match. Deferred `DUMMY` tags account for the remaining scored
BE differences before materialization (including seven fields at the jungle
FEATURES boundary); WG presence still differs after native FULL conversion.

Verification completed with all three edge-callback tests passing and the full
workspace command:

```text
cargo test --workspace --release --locked --no-fail-fast -j 1 -- --test-threads=2
```

Saved result: `target/generation-checks-20261001/tests-1791086807931819700.{json,log}`.
The preceding attempts remain preserved: one exceeded the initial one-hour limit,
and one hit Windows' lock on an already-running test executable. The final run
used serialized verification and the larger time allowance; it completed normally.

Evidence directories under `target/`:

- `full-parity-20261003/trial-andesite-trace-1791082183184295100/`
- `full-parity-20261003/andesite-trace-comparison-01.json`
- `full-parity-20261002/native-history/pot-integrated-1791082607744111900/`
- `full-parity-20261003/scattered-history-native-1791083227866856600/`
- `full-parity-20261003/scattered-history-verified-01.json`
- `full-parity-20261003/scattered-runtime-1791086089277161100/`
- `full-parity-20261002/native-history/structures-integrated-1791086810008684800/`

SPAWN/FULL runtime conversion, deferred block-entity materialization, tick-container
lifecycle, remaining structure/features and saved-holder hydration are incomplete.
Native FULL removes WG maps; the current unconverted BCore chunks retain them.
The spawner component independently covers all 19 overworld CREATURE finalizers,
but its runtime/environment/entity-storage hookup remains required.

## Shared-context snapshot — 2026-10-02

The freshly built release diagnostic now uses one `GenerationWorld` for all chunk
coordinates intersecting each captured region, in lexicographic request order.
This run used **four Rayon workers**, the same seed and the three saved vanilla
captures described below. [parity-live-generation.json](parity-live-generation.json)
records the executable/capture hashes, differences and actual source coverage.

| Centre | Terrain heights | 3D biome cells | Exact block states | Change from historical snapshot | Lowest-log proxy V / B / intersection |
|---|---:|---:|---:|---:|---:|
| (0,0) | 256/256 | 1536/1536 | 92242/98304 | −592 | 12 / 8 / 1 |
| (1000,0) | 256/256 | 1536/1536 | 96618/98304 | −413 | 16 / 11 / 2 |
| (-2000,3000) | 256/256 | 1536/1536 | 97187/98304 | +405 | 13 / 9 / 5 |

Combined exact-state match is **286047/294912 = 96.99% including air**:
**600 fewer matches** than the historical 97.20% checkpoint. This is a regression
in that integration metric. Component parity and successful source execution do
not establish correct placement under the live server's native request history.
All **768 terrain heights and 4608 quart-cell biomes** still match.

The three diagnostic invocations requested **8 chunks** and recorded **40 distinct
FEATURES sources**, **2197 completed placed streams** and **138 missing streams**.
Completed streams may legitimately place no blocks. All eight targets finished
their supported incoming-source attempts; **none has complete native coverage**.
The missing placed operations in this sample are `amethyst_geode`,
`underwater_magma`, `freeze_top_layer`, and, in neighbouring source palettes of
the second region, `bamboo_light` and `vines`. Coverage also names missing
non-mineshaft structures, density adaptation, noise ore veins and final stages.

### Integration fixes and focused regressions

- Biome admission now bridges BCore wire/save IDs to the native catalog by
  resource-key identity. **13932 biome/feature memberships**, including shifted
  IDs, and a real swamp-seagrass placement regression verify this boundary.
- Glow lichen uses the native-supported multiface predicate for cache-null shapes:
  **65 proven states / 195 native observations**, with **114 contextual states**
  still explicitly unsupported. **10 cave and 6 sculk reference tests passed**.
- The tall-mangrove write-order failure and ancient-city inventory-NBT fixture
  failure are resolved. The release **extra-tree, generation and terrain filters
  passed all 30 tests**; the exported **jigsaw reference suite passed all 5**.
- Extra trees now have **688 native cases across 24 variants**, replayed on both
  caller-owned RNG backends. **35 tree tests passed**, including four native
  HashSet collision-bin traces with **1152 operations**. A fresh Java run reproduced
  the original configured-tree fixture and catalog; the collision probe records
  its separate source/helper hashes. Natural pale-oak callbacks, contextual edge
  effects and Java identity-hash ties remain outside this coverage.
- The jigsaw suite checks **70 native cases**: 24 assemblies / **2527 pieces**
  (ancient city and five village variants), 28 template placements, 12 transforms
  and 6 city-placement cases. The corrected native probe initializes item data
  components before inventory decoding; it also exposed and verified the Rust
  correction to omit `LootTableSeed` when a saved fixed inventory has no loot table.
  The city cases include **203367 writes and 6384 postprocessing marks** in
  controlled worlds. Runtime starts/references, density adaptation and the generic
  template block-entity/entity handoff are still required.
- NOISE caches now live for a Rayon job's fixed graph/context rather than being
  discarded after every column. Four terrain regressions verify cold-column
  equality, density bits, seed/context isolation, worker/unwind cleanup and bounded
  retained capacity. Numerical behavior is checked before reporting a speedup.

### Density-cache measurements

Release measurements on a 16-logical-worker host used explicit one/four-worker
Rayon pools and excluded compilation. The 7×7 workload requested every chunk in
`[-3,3]` in Z-then-X order through one `GenerationWorld`, seed
`846692123413862008`; it excluded protocol encoding and disk I/O.

| Workload | Workers | Before | After |
|---|---:|---:|---:|
| NOISE phase, 25 chunks, seed 1234 | 4 | 17.324 s | 4.282 s |
| Entire phase diagnostic, same dependency envelope | 4 | 19.239 s | 5.077 s |
| Direct FULL request, seed 1234, (0,0) | 1 | 40.863 s | 8.270 s |
| 49 adjacent FULL requests | 4 | 127.612 s | 44.561 s |

The before runs overlapped other work, so the observed ratios are **indicative,
not controlled causal benchmarks**. The independently counted 64-column probe
produces **24576 bit-identical density values** with **24576 → 1536 corner
evaluations**. Actual Rayon job sizes vary; this is not a 16× end-to-end claim.

The complete one-worker FULL result (chunk plus coverage) matched its 423248-byte
baseline byte-for-byte, SHA-256
`6fd7500e1579e170786a68260f28af2f2ea9fb856da2216fed38313ca0e7991e`.
The aggregate fingerprint of all 49 grid results also remained unchanged:
`959869bb0fc00dd377f0d11d9c550762e783d65b699733829c5193d3244013d1`.
Repeated FULL requests reused existing work. Fifteen focused terrain/numerical/
reference tests passed, including the 62649-value numeric oracle and saved
height/biome references. NOISE remains the largest measured phase; peak memory
and default 16-worker scaling were not measured.

## Live generation integration — 2026-10-01

`GenerationWorld` now retains source work and neighbouring effects across requests.
Server `World`, its clones and queued jobs use this shared context, as does each
multi-coordinate `dump_chunk` invocation. Diagnostic output includes stage/source
coverage; `World::try_generate` exposes the same coverage to server callers.

The runtime checks caught two integration failures: a stale base-feature catalog
without `shape_ranges`, and a missing Gaussian-cache delegate that aborted sources
at `pointed_dripstone`. The catalog was recaptured from the pinned JAR, and placement
now uses the cave RNG's retained cache. **7 base-feature tests and 10 generation
tests passed**, including mixed forest/dripstone sources and repeated/concurrent
generation.

Typed sculk block entities and unconsumed postprocessing marks now survive
`GeneratedChunk` → `ChunkColumn` → `.bcc` v4. **106 native-compatible sculk states**
round-trip with exact typed data and empty client updates. **17 targeted protocol
tests passed** across generation context, sculk effects, tree effects, dungeon data
and queued entity delivery. See [tree-effects.md](tree-effects.md) for the format.

This is a live integration milestone, not FULL completion. Persisted holder
history, saved-neighbour hydration, native multi-task schedule matching, remaining
feature/structure kernels, lighting, spawning and tick execution remain incomplete.
[feature-scheduling.md](feature-scheduling.md#current-bcore-runtime) describes the
current runtime and its boundaries. The older snapshot below retains its historical
binary/capture provenance.

## Tree effects, tick containers and dependencies — 2026-09-29

- `finish_chunk` now transfers staged hive snapshots and raw tick requests into
  owning region chunks. The returned chunk preserves them through **`.bcc` v4**;
  readers accept v1–v4. Hive type **34** sends empty client update NBT (`0a00`),
  while occupants and requests remain server-side.
- **100 native hive samples / 48 states** and **22 native tick-container traces**
  cover generated metadata and chunk-local queue semantics. Eight Rust tick tests
  also check preparation, conversions, ordering, filters and priority handling.
  [tree-effects.md](tree-effects.md) records the contracts and reproduction commands.
- The [native dependency graph](feature-scheduling.md) covers **12 statuses,
  24 generation/loading steps and 156 task-layer radius queries**. Metadata does
  not establish a global source order; shared status-aware chunks and native
  multi-task schedule captures remain prerequisites for global vegetation hookup.
- Independent native `--verify` passed for the hive, tick and graph fixtures.
  An additional **18 actual WorldGenRegion rewrite traces** caught and verified the
  fix for losing occupants on same-state hive writes, including staged data.
  **28 region, 8 tick-container, 18 storage and 21 protocol integration tests passed**
  in this follow-up. The latest saved full worldgen run records **182 passed /
  2 ignored**. That checkpoint had no confirmed complete post-change protocol run; see
  [regression suites](#regression-suites).

The tick helpers currently have fixture-test callers. Runtime handoff, clock and
sub-order allocation, cross-chunk dispatch and actual gameplay ticks still need
implementation. Target-local finalization still drops the other region-owned
chunk copies; owning-chunk transfer alone does not solve cross-request scheduling.

## Numerical/component update — 2026-09-25

- The primary inner loop is now **62,649 bit-exact Java/Rust values in 895 cases**,
  covering f32/f64 primitives, all 62 NormalNoise parameter sets, density graphs,
  raw router fields and native NoiseChunk interpolation/cache wrappers.
- The initial 851-case / 59,633-value matrix found 2,570 differences. Float-accurate
  splines and the native ImprovedNoise epsilon resolved them; expanded cases also
  verified cache semantics, configuration cache keys and clamped-gradient endpoints.
  Reverse input order and four workers preserve exact results. Local checks take
  roughly half a second.
- Constructor-hoisted Perlin constants retained exact values and sped up the local
  erosion-noise microbenchmark by about **1.8×**. This is a kernel timing.
- [numeric-parity.md](numeric-parity.md) provides reproducible commands, custom
  JSON requests and editable fade/lerp experiments. Algebraically equivalent FMA
  and expanded-polynomial candidates were measured and rejected for bit differences.
- Full worldgen regression run at this numerical milestone: **123 passed,
  2 ignored**. Saved height/biome reference checks and the ore/tree/dungeon/mineshaft
  component fixtures passed.
- **Pre-streaming protocol baseline: 178 passed** — 143 library unit tests,
  5 flat-format tests, 15 persistence tests, 11 terrain tests, 2 dungeon tests and
  2 mineshaft-persistence tests. This predates the latest entity-streaming changes.
  Some combined attempts initially exceeded a **360 s** outer timeout; the slow
  targets subsequently completed separately (persistence **173 s**, terrain
  **462 s**). An interrupted attempt is not counted as a completed pass.
- Workspace release build, formatting and diff checks passed at that earlier
  stage. The three unused login UUID helpers have since been removed; the targeted
  streaming run at that point emitted **no compile warnings**. Later results are
  recorded under [regression suites](#regression-suites).

The bundle is pinned to the actual 26.1 JAR. A previous extracted datapack contained
26.2 density/climate data, including `sulfur_caves`. `bundle_worldgen.py` now reads
noise/density/settings directly from the pinned JAR and uses **7,593 native climate
rows / 65 native biome IDs**. The existing **66-entry BCore wire/save ID map** is
retained independently; it is not the target generation's biome list.
Compare biomes by resource-key name: for example, `windswept_gravelly_hills` is
BCore ID **62** but native ID **61**. A raw-ID difference across those registries
does not by itself establish a generation-biome mismatch.

## Shared native Mth lookup

The native Mth fixture verifies **65,536 table entries** and **1,903 angle cases**
(898 f32-promoted, 1,005 f64), yielding **3,806 sine/cosine outputs** including
NaNs, infinities, saturation and index boundaries, with **zero bit mismatches**.
The JAR exposes **only double-argument overloads returning float**; f32 inputs widen
before scaling. Three Rust tests passed, including shared-table checks with
**eight threads**. The full table's little-endian f32 SHA-256 is recorded in
[numeric-parity.md](numeric-parity.md#shared-native-mth-lookup).

The **256 KiB `OnceLock` table** is now used by carvers and ore-radius Mth calls.
At the lookup-integration stage, **10 carver + 3 Mth + 2 ore tests passed**. On a
Ryzen 7 5700X under Windows release, **16,384 inputs × 256 iterations × nine
alternating rounds** measured **11.14×–12.67×** improvements in the four sine/cosine helpers,
with cold initialization measured separately at **385.2 µs**.
These are per-call measurements, not whole-chunk speedups.
[numeric-parity.md](numeric-parity.md#shared-native-mth-lookup) records all four
before/after timings, native signatures and reproduction commands. The earlier
62,649-value / 895-case matrix and ~1.8× Perlin result retain their original scope.

## Generated-entity streaming update

Generated chest minecarts now travel from generated or loaded chunk data into the
shared payload cache and each player's view. Spawn packets follow the completed
chunk batch, once per delivered chunk. Leaving a chunk removes its entities;
teleporting retires the old chunks and sends removals/unloads before any resend,
including a teleport within the same chunk. Delivery bookkeeping commits only
after a successful stream write.

Runtime IDs come from a process-wide atomic allocator starting at **393**, above
the reserved local-player ID **392**. Cache entries and active views share `Arc`
entity records through a pruned weak index. A UUIDv8 hash of the world seed, owning
chunk, row ordinal, type, initial position and loot seed reconstructs identity
from unchanged ordered generated data (the layout introduced in `.bcc` v3),
independently of placement RNG. This contract is for immutable **GENERATED /
pre-gameplay** records; persistent mutable entity identity, motion and inventory
state remain future work.

The confirmed native packet fixture now contains **27 samples** from
`scripts/EntityPacketReference.java` and
`crates/bcore-protocol/data/entity_packets_26_1.json`: static spawn, removal,
metadata from a real minecart, and absolute teleport packets. Fixes include the
one-byte zero `LpVec3`, the full teleport position/delta/rotation/relative-field
layout, and native type IDs (item **71**, zombie **150**, cow **30**, chest minecart
**25**). The original 26 samples are retained, with an added actual
`MinecartChest.getAddEntityPacket(ServerEntity)` sample. Independent native capture
with **`--verify` passed**, and the latest confirmed Rust
**`tests/entity_packets_reference.rs` test passed** against all 27 samples.

The initial **seven lifecycle unit tests passed**, covering spawn ordering,
duplicate suppression, unload, same-chunk teleport, multiple viewers/cache eviction,
UUID reconstruction and duplicate row ordinals, concurrent cache publication,
and failed-write state. The latest confirmed targeted **`world::` run passed
26 tests**, including **four new adoption regressions**. The newer complete
protocol run passed **234 tests**, including these lifecycle checks.

The coordinate-only `mark_loaded` API was replaced by an opaque, non-Clone
`ChunkDeliveryState`. `previous.into_delivery_state()` consumes the old view;
`destination.adopt_delivery_state(token)` moves its exact loaded/retired chunks,
entity `Arc` ownership and teleport-ID sequence. A destination with existing
delivery or issued-teleport state is rejected unchanged, returning the intact
token. The caller must ensure both views serve the **same connection**.

The `.gitignore` save rule now uses `/world/` and retains `/crates/*/world/`, so
`src/world/entity_tests.rs` is no longer hidden by the old broad `world/` rule.
[entity-streaming.md](entity-streaming.md) describes the ownership model, adoption
regressions and reproduction commands. Minecart motion, loot unpacking/inventory
interaction and spawner simulation remain incomplete.

World-specific queue routing is now fixed. `World` wraps `Arc<WorldInner>`;
`GenerationJob` retains the requesting world, with per-world in-flight reservations
and RAII cleanup on completion, discard, send failure or unwind. One dispatcher
services the queue. **Six queue tests and 26 world tests passed** in targeted runs;
the later 234-test protocol result includes the complete library and integrations.

## Historical whole-chunk snapshot

The checked-in `parity-results.json` records the executable-specific snapshot
below. Its three exact-state counts match the earlier numerical/mineshaft
checkpoint; the executable hash below comes from that JSON. This is an integration
check on these captures, not evidence that numerical
rounding errors are harmless on other coordinates/seeds. Later targeted entity,
tree-effect and tick-container checks do not refresh this snapshot or establish
results for a newer executable.

- Seed: `846692123413862008`; protocol: **775** (intentional development target).
- Regions centred at `(0,0)`, `(1000,0)`, `(-2000,3000)`.
- Full vertical range: **Y=-64..319**, 98,304 block states per region.
- Vanilla: live offline server 26.1 at `127.0.0.1:25571`.
- BCore: freshly built `dump_chunk` release executable, not the running server
  or saved BCore chunks. Vanilla captures are reused across comparisons.
- Reference: `crates/bcore-worldgen/data/parity_26_1.json` contains 768 terrain
  heights and 4,608 quart-cell biomes plus hashes of the source captures.
- Raw measurements and binary/capture hashes: [parity-results.json](parity-results.json).
- Recorded snapshot's `dump_chunk` executable SHA-256:
  `8c4f15bbc4c2da665fd41c9d901520a2201081def0410f6aca9ee1523fb02c6d`.

### Results

| Centre | Terrain heights | 3D biome cells | Exact block states | Lowest-log proxy V / B / intersection |
|---|---:|---:|---:|---:|
| (0,0) | 256/256 | 1536/1536 | 92834/98304 (94.44%) | 12 / 5 / 1 |
| (1000,0) | 256/256 | 1536/1536 | 97031/98304 (98.71%) | 16 / 6 / 1 |
| (-2000,3000) | 256/256 | 1536/1536 | 96782/98304 (98.45%) | 13 / 5 / 1 |

Combined block-state match: **286647/294912 = 97.20%**. Air is included, so this
number must not be presented as the completion percentage of the generator.
State comparison includes properties; biome comparison uses names, since the
transitional BCore registry capture and vanilla 26.1 assign some different IDs.
The lowest-log proxy counts branches and fallen logs too; it is not a tree census.

### Underground-generation checkpoint

The ore region and aquifer corrections add **964, 299, 1770** matching states
relative to the preceding double-precision baseline: **+3033 total**. Of these,
the aquifer correction adds **836** in the third region. That comparison
had **8265 differing states**, including vegetation, cave features and
structures. The following component checks do not imply complete world parity:

- `ores_26_1.json`: **392** native configured OreFeature cases, checking target
  tags, block writes, return values and subsequent RNG. Includes cave air
  handling, water, tuff, deepslate axes, build-height and chunk boundaries.
- `ore_placements_26_1.json`: **360** native modifier-plus-shape streams for
  **30** overworld ore placements on stone, deepslate and a cave-plane world.
  Covers count/rarity, height distributions, lazy placement and exposure RNG.
  Biome admission is isolated from this fixture. Clay, extra badlands gold,
  emeralds and infested blocks are now included at their native step/index.
- `biome_zoom_26_1.json`: **88434** block-to-quart selections through the actual
  BiomeManager, across six seeds, negative coordinates and world-border positions.
  Ore biome checks use the hashed-seed jittered selection. The new region-aware
  tree selectors also use region biome admission; global vegetation scheduling
  remains pending.
- `feature_order_26_1.json`: all **11** native decoration steps, built from the
  native overworld source's **54** possible biomes, match the Rust sorter.
- `aquifers_26_1.json`: **84** full-height columns at three input densities,
  four seeds and seven locations (**32256** substance/update-flag checks), plus
  **16** aquifer-center samples. These use native RandomState and NoiseChunk.
  Corrected positional factory derivation, center-coordinate distance/status
  queries, equal-distance ordering, surface-fluid early returns, deep-dark dry
  regions, fluid-level quantization and lava sampling. Fluid update flags are
  computed and tested; scheduling them in the server remains separate work.

Ore neighbour reads use actual pre-feature terrain rather than clamped target
heights. A bounded 64-entry immutable terrain/carver cache supplies copy-on-write
region chunks; feature writes cannot contaminate cached terrain. `dump_chunk`
supports several coordinate pairs per process so adjacent diagnostic chunks
reuse that cache. Source-chunk replay is still a bounded approximation to the
complete native dependency schedule, and earlier structures/features remain
missing from that region.

At that stage, the full worldgen run had **111 tests passed, 2 ignored** with
`cargo test --release --locked -p bcore-worldgen --no-fail-fast -- --test-threads=4`.
Native ore, placement, biome-zoom, feature-order and aquifer fixtures were
regenerated/verified against the pinned 26.1 JAR. The result table is from a fresh
release diagnostic binary; its hash is recorded in `parity-results.json`.

### Carvers and production surface resolution

The latest `carvers_26_1.json` capture has **1,431 cases**: the prior **498**,
**899 surface evaluations**, **26 additional surface-voxel cases** and **eight
badlands palettes**. Coverage includes all **54 overworld source biomes**,
**451 evaluations through the production adapter**, **13 new voxel cases using
native aquifers**, and **1,536 palette states**. Independent native
`--probe carver --verify` passed; the merged **13 carver tests passed in 0.25 s**
(`target/carver-merged-check.log`).

Production `top_material` now evaluates the real surface-rule resolver, including
raw-generator biome selection and native preliminary-surface, depth, water and
temperature conditions. The required bundled
`crates/bcore-worldgen/data/surface_climates_26_1.json` supplies native climate
values. General custom heights, retrogen, nonempty blending, fluid-tick execution
and complete mixed-feature region comparisons remain outside this coverage.

#### Earlier 498-case kernel/voxel checkpoint

`scripts/CarverReference.java` and
`crates/bcore-worldgen/data/carvers_26_1.json` first established **498 native
samples** for `cave`, `cave_extra_underground` and
`canyon`. All **432 original samples** are preserved, with **66 bounded voxel
cases** added:

| Probe | Native samples | Checked output |
|---|---:|---|
| `canReach` | 296 | Boundary decisions, including translated negative/world-border positions |
| Source seed/start | 96 | Carver-index seeding, admission and RNG continuation |
| Cave thickness | 8 streams | 128 f32 values and RNG continuation |
| Canyon widths | 8 streams | 3,072 f32 values and RNG continuation |
| Geometry | 24 traces (16 carve, 8 tunnel) | 3,398 ellipsoids with bit-exact positions and radii |
| Bounded voxel operations | 66 cases | Native ellipsoid/block carving, skip rules, masks, writes and postprocessing |

The replaceability table separately checks **29,873 registered states × 3
configurations = 89,619 predicates**. Each configuration accepts the same **87
states**. Native f32 additions followed by f64 squaring correct **74 reach
disagreements**. The fixes also restore **72 missing accepted states**, preserve
wrapping `world_seed + carver_index`, and compute canyon length with native f32
multiplication before conversion to an integer. Seed
**`87921060185259`** is the canyon-length witness. At the original geometry-only
stage, **six targeted Rust carver tests passed in 0.14 s**.

The original 24 geometry traces override only `carveEllipsoid` to record arguments
after the JAR performs room, tunnel, branch and canyon calculations. The 66 voxel
cases instead execute the actual native **`carveEllipsoid`, `carveBlock`, skip
predicates, `CarvingMask` and `ProtoChunk` setter**. Instrumentation records their
results while delegating to those implementations.

Aquifer replies and surface returns are explicitly scripted in these voxel cases:
`aquifer.mode = "scripted_cycle"` and `surface.mode = "scripted_constant"`. The
verified counts across the 66 cases are:

| Observation | Count |
|---|---:|
| Ellipsoid operations | 64 |
| Direct block-carving operations | 17 |
| Mask reads / writes | 943 / 903 |
| Final mask bits, summed across cases | 906 |
| Block callbacks / successful callbacks | 920 / 741 |
| Scripted aquifer calls | 830 |
| Fluid-scheduling flag queries / true results | 741 / 384 |
| Write requests / state-changing writes | 748 / 746 |
| Postprocessing marks | 310 |
| Scripted surface callbacks | 15 |
| Block callbacks reporting the sticky surface flag as true | 18 |

Cases may contain multiple operations. Final mask bits are summed per case rather
than deduplicated across different cases. Corrections include native X/Z traversal,
wrapped mask coordinates, sticky per-column surface flags and callback behavior,
preservation of duplicate postprocessing marks, and retention of the aquifer's
preceding scheduling flag on direct-lava paths. The expanded **10 carver tests
passed in 0.18 s**, and independent native recapture with **`--verify` passed** at
this checkpoint. Both carver timings are test-run measurements, not generation
benchmarks.

These historical 66 voxel cases retain their scripted environment. The newer
surface evaluations and native-aquifer voxel cases extend that evidence without
changing the earlier counts. The shared Mth lookup remains integrated.

### Monster rooms and block-entity persistence

`MonsterRoomReference.java` runs the actual 26.1 MonsterRoomFeature and saves
native chest/spawner block entities. The **144 cases** in
`monster_rooms_26_1.json` cover six RNG seeds, three origins (including negative
coordinates/build-height edges), tunnels, sealed/over-open rooms, floor and roof
holes, unsupported floors, protected blocks and existing chests. Rust matches
changed-block hashes, return values, RNG continuation, full generated NBT values,
client update data and block-entity type IDs. The flag table comes from native
`isSolid`, `isSolidRender`, air and replacement-tag predicates.

Both `monster_room` and `monster_room_deep` run in generation step 3, before ores,
using region-backed biome admission. Generated block entities follow writes into
neighbour chunks. A separate negative-coordinate region test checks the native
room's chest/spawner data across chunk boundaries and immutable-cache isolation.

Chunk packets now include chest/spawner entries and anonymous update NBT;
server-only loot seeds are retained on disk, not sent in chest update data.
The `.bcc` writer now uses format version **4** and the reader accepts versions
**1–4**. Generated minecarts and structure metadata retain their v3 layout; v4 adds
[hives and raw tick requests](tree-effects.md#deferred-requests-and-bcc-v4).
Removing a container removes its metadata. Native `cave_air`/`void_air`
are treated as air in heightmaps, lighting surfaces and section block counts.

At the dungeon-only stage, **48 targeted protocol tests passed**: flat packet fixtures (5), persistence
integration (15), varied terrain encoding (11), storage unit tests (15) and native
dungeon block-entity/NBT tests (2). The old terrain-variation test sampled ten
ocean water surfaces at Y=62; it now uses the independently captured forest seed.
Its non-air count check includes all three native air states.

The three main sample counts are unchanged by dungeon integration. This does not
establish native dungeon locations: multi-feature/native-region captures, earlier
structures and full source-chunk scheduling remain required. Loot opening and
spawner gameplay/ticking are also separate from generated-data compatibility.

### Mineshaft admission, placement and persistence

`MineshaftReference.java` now invokes the native public `findGenerationPoint`,
including its discarded legacy probability draw and terrain-dependent height
adjustment. **36 cases / 5714 pieces** match the Rust layout: six seeds (including
both signed 64-bit extremes), three chunk positions, and normal/mesa types.
Checks include every piece's bounds, orientation, generation depth, room entrance
boxes, rail/spider flags, the returned generation point and RNG continuation.

Normal mines use native below-sea-level adjustment. Mesa layout tests use native
WORLD_SURFACE_WG heights; BCore's base-height query is separately checked against
those native noise columns. The start retains the discarded legacy `nextDouble`
and the generation point's minimum-Z coordinate.

- **618 postProcess cases** check block hashes, RNG continuation, clipped mutable
  piece state, cave-spider NBT, real chest-minecart loot/position/type and native
  postprocessing marks. Coverage includes rotations, normal/mesa materials,
  environmental rejection, pillars/chains and forward/reverse clipping.
- **136 `createStructures` cases** check frequency, weighted retries, biome/height
  admission, bounds and piece hashes. Native structure-step indices are retained.
- **Two overlapping solid-terrain regions** check reference iteration order,
  generated blocks and entities, both together and as independently requested
  chunks in reverse order. Earlier spider-corridor clips are replayed explicitly.
- **144 native shape cases** check fence connections and wall-torch survival.

The 618 piece cases, 136 admission cases, two solid-terrain regions and 144 shape
checks were freshly verified against the pinned JAR. The region fixture's source
hash was refreshed with **identical samples**; this is a provenance refresh, not
an improvement in generated output or additional mixed-feature coverage.

Mineshafts now run in the feature region before monster rooms/ores. Writes and
metadata reach owning chunks without changing cached terrain. `.bcc` v3 persists
loot minecarts, cave-spider spawners, layouts and references, and validates counts,
bounds, coordinate ownership, kinds and trailing data. Native entity UUIDs are
independent of placement RNG and excluded from deterministic generated-data checks.

Static generated minecarts now use the streaming lifecycle described above.
The bounded replay remains an approximation to the native dependency schedule;
mixed-feature regions, other structures and general neighbour/fluid updates still
need work.

### Region-aware standing trees and selectors

`standing_trees_26_1.json` covers **438 full native cases**: **378 configured-tree
cases + 60 placed-selector cases**, spanning **13 standing variants and eight
placed selectors**. Both RNG backends match native draws, direct write flags,
leaf/edge updates, bee data and tick requests on the live absolute-coordinate
copy-on-write region. `standing_blocks_26_1.json` covers all **29,873 states**.
Native capture and independent verification passed for both new fixtures.

**127 independent target replays**, including **63 incoming-only targets**, match
under **explicit fixed source plans**. Each target replays the complete supplied
plan; this does not supply the native global source scheduler. Jungle variants and
context-dependent edge effects remain explicitly unsupported.

At the standing-tree checkpoint, the targeted result was **45 passed**:
5 standing-region, 18 tree, 4 decoration, 5 fallen-region, 4 bounded-vegetation-region
and 9 fallen-reference tests. The older **37 standing**, **655 fallen** and
**504 bounded placement-driver cases** remain preserved. The historical driver is archived at
`scripts/vegetation-bounded/VegetationReference.java`; the root
`scripts/VegetationReference.java` now probes full standing trees/selectors.

Global region vegetation is **not hooked up**. Staged hive snapshots and tick
requests now transfer into their owning chunks during `finish_chunk`, with the
returned chunk retained through `.bcc` v4 save/load and queued loading. Transfer,
retry and replacement behavior are covered in [tree-effects.md](tree-effects.md).
Native full-source scheduling, cross-request ownership and runtime tick execution
remain pending.

### Corrections to the previous report

The old grid harness compared X-major vanilla output against Z-major BCore output.
Its reported 16–59% terrain match was a transposition error. Coordinate-addressed
comparison shows **768/768 height matches already in the baseline**. This release
does not claim to have repaired that terrain.

The previous report labelled Y=40..120 while actually sampling 40..220. The new
snapshot records explicit bounds, complete coordinate coverage and executable /
capture hashes. Empty biome names from Mineflayer are resolved using the live
configuration registry, rather than treated as valid biome measurements.

Early baseline exact-state counts were 91653, 96153, 94766. The alpha.1 tree changes
improved two regions and worsened `(0,0)`; total improvement was only 101 states.
This is evidence of remaining tree-shape and inter-chunk dependencies, not parity.

### Post-alpha.1 tree-clearance check

The clearance-only stage added the configured two-/three-layer `minimum_size` scan
and optional clipped height before any trunk writes. Rejected trees stop after
dimension sampling, preserving the RNG position for the next placement attempt.
The upper world boundary permits `origin_y + tree_height + 1 == 320`.
Configuration values and control flow were checked directly against the 26.1
server JAR (`TreeFeature.doPlace`, `getMaxFreeTreeHeight` and configured features).
That pipeline used chunk-local clearance. The current region-aware implementation
and its broader fixture coverage are described above.

The clearance-only change versus alpha.1 was **0, 0, +30**. The third
region's lowest-log intersection decreased **4 → 3**, so this is not evidence of
tree-origin parity. The table and JSON above describe the working-tree build,
not the published alpha.1 binary.

The neighbour-spill experiment was removed: it changed exact-state counts by
**−39, −21, −102** relative to alpha.1. Replaying only nearby attempts skips RNG
consumed by other trees and cannot reproduce a shared generation region.
Removing it restored all three baseline measurements before the clearance fix.

The index-search scripts also omitted RNG draws between trees. Their overlap
scores cannot identify a feature index or disprove the seed. The Python RNG
probe now prints only the first candidate, before admission, and imports silently.

Validation at that stage: `cargo test --release --locked -p bcore-worldgen --no-fail-fast -- --test-threads=4`
passed **98 tests**, with **2 ignored**, including clearance, leaf-update and placement-height tests.
`cargo fmt --all --check` and `cargo build --workspace --release --locked` passed.
Saved captures still match **768/768** terrain heights and **4608/4608** biome cells.

### Isolated vanilla tree reference and leaf updates

`scripts/TreeReference.java` calls the actual 26.1 `TreeFeature.doPlace`, decorators
and `updateLeaves` implementations in a minimal in-memory world. Configurations
are decoded from the JAR's JSON using its native codec, and block tags are loaded
from the same JAR. The saved fixture is
`crates/bcore-worldgen/data/trees_26_1.json`, with source/JAR hashes, full-chunk
state hashes before and after leaf updates, and the next RNG long.

The initial **16 cases** (oak, birch, spruce, pine × seeds 0, 1, 17, 42) matched
the final state hash and next RNG value. This older fixture retains **37 cases**,
including the extensions below. Its scope excludes selectors, adjoining trees,
chunk borders, biome placement, shape-edge updates and scheduled ticks; the newer
438-case fixture covers the supported full region-aware tree/selector paths.
Fixture seeds initialize feature RNG directly, not a world seed.

The leaf pass records written blocks and processes distance buckets within the
tree's bounds, after decorators. It preserves persistence/waterlogging properties.
Vanilla's hash-bucket iteration and queued entries in multiple distance buckets
matter: a conventional shortest-distance BFS fails the JAR reference even for an
isolated oak. The compact position set reproduces chained hash buckets and resize
ordering; Java treeified collision buckets are not implemented or covered here.

This step adds **21, 23, 54** matching states to the clearance baseline (**+98**
total), without changing the name-level differences or tree-origin counts.
The leaf-update snapshot had **+128** exact matches relative to alpha.1.

Three sequential process-level timings for chunk `(62,0)`, same world seed:
before leaf updates **4.163, 4.122, 4.614 s**; after **4.165, 3.766, 3.669 s**.
These include startup and JSON output, and indicate no obvious slowdown; they
are too few/noisy to establish a speedup. Worldgen performance remains unfinished.

### Live tree-placement heightmaps and admission order

Tree placement now scans current chunk blocks for `WORLD_SURFACE` and
`OCEAN_FLOOR`, rather than using the pre-feature terrain height. Both maps return
the first free Y above their respective predicates. Leaves count towards
`OCEAN_FLOOR`; fluids and non-solid ground cover only count towards `WORLD_SURFACE`.
The depth filter compares those heights (maximum 0 for the common tree features,
2 for swamp), and can therefore also reject grass/litter above the floor.

The bundled `data/heightmaps_26_1.json` contains run-length encoded predicates for
all **29,873** vanilla state IDs: non-air, blocks-motion and `supports_vegetation`.
They are extracted by `scripts/HeightmapReference.java`, then expanded once to a
29 KB lookup table. Heights are read live, so tree writes/removals do not leave a
stale placement cache. The snapshot's terrain-height metric retains its original
meaning (before features).

The old grass-only/sea-level admission test has been replaced with sapling support.
Forest/taiga/savanna/jungle/windswept selectors consume their selection draws before
the chosen child's soil filter. Features with an outer sapling filter still reject
before selection. This fixes the RNG position after blocked forest attempts.
At that stage, tree admission used placement Y without region biome zoom. The
current region-aware selector path includes it.

**40 reference columns** cover empty space, grass, dirt variants, moss, mud,
farmland, sand, vegetation, fluids, leaves and all air variants at Y=40 and Y=64.
The probe evaluates the actual JAR heightmap predicates and native sapling
`canSurvive`; it does not invoke a full chunk generator. Both this fixture and the
isolated-tree fixture were regenerated and verified with `--verify`.

Versus the leaf-update snapshot, exact matches increased **+527, +79, +61 = +667**.
At that stage the improvement versus alpha.1 was **+795**, with lowest-log intersections
**1, 1, 2** (previously **1, 1, 3**): fewer incorrectly admitted trees improves the
block score, but missing neighbour trees and incomplete variants still dominate
tree-position differences. No 100% worldgen claim follows from these measurements.

### Fancy oak, below-trunk provider and decorator base

The fancy oak now has its own trunk placer: branch-cluster sampling, clearance
checks, float-rounded limb rasterization, per-block log axes and rounded foliage
rows. The former straight-trunk/square-foliage substitute is removed.

The default below-trunk rule now preserves the JAR's
`cannot_replace_below_tree_trunk` tag (11 state IDs in this version), replacing
other blocks with dirt. Protected dirt is not recorded as a newly placed trunk
block. Soil probes cover dirt, coarse dirt, podzol, moss, mud, rooted dirt,
farmland, stone and air. Stone/air cases invoke the tree feature directly; outer
sapling admission is tested separately and still rejects those substrates.

`PlaceOnGroundDecorator` now takes the bounding rectangle of the lowest actually
placed trunk blocks. This includes the replaced dirt block when the below-trunk
provider writes it. Using the original sapling Y shifted the litter attempts and
all subsequent RNG draws. Native decorated-oak fixtures failed before this fix
and now pass, including fancy oak with litter.

All **37 fixture cases** match full-chunk state hashes and the next RNG long:
28 cases across seven configurations and four seeds, plus nine soil cases.
The eight litter cases invoke native decorators but exclude successful beehive
placement, now covered by the newer region fixtures. At that stage the Rust
worldgen suite had **98 passing tests / 2 ignored**; fixture cases run inside one test.

Compared with the live-heightmap baseline, this step adds **119, 24, 2 = 145**
exact matches. Total gain versus alpha.1 is **940** states. Lowest-log intersections
are **1, 1, 1**, down from **1, 1, 2**. Those measurements used the chunk-clipped
pipeline, where border clipping changed reads, decorators and later random draws.

### Isolated fallen-tree feature

`scripts/FallenTreeReference.java` now captures **655 native cases** in
`crates/bcore-worldgen/data/fallen_trees_26_1.json`: **131 each** for oak, birch,
super birch, jungle and spruce. All **10 original cases** and their results are
preserved. The isolated Rust implementation in `tree/fallen.rs` matches complete
world-space writes, ordered read/write traces, write flags and acceptance,
placement return values, subsequent RNG and postprocessing requests. All **nine
Rust tests** in `tests/fallen_tree_reference.rs` passed.

Coverage includes negative/chunk-edge/world-border positions, build-height
rejection, multiple soils and cached support shapes, gaps, obstacles, decorators
and rejected writes. Postprocessing checks compare requested positions; subsequent
neighbour updates and ticks are outside this isolated fixture.

#### Live FeatureRegion adapter

The live adapter now passes the **same 655 native cases** through `FeatureRegion`:
**645 writable cases + 10 explicit write-failure injections**. This reuses the
original fixture rather than adding another 655 native inputs. Checks cover
ordered reads/writes/flags, return values, RNG continuation, owning-chunk contents
and marks, and preservation of the immutable base chunks. At the fallen-adapter
stage, **eight region tests passed, including five new tests**; **nine fallen-tree
and 18 tree tests** also passed. The later 45-test tree result is recorded above.

The adapter borrows the caller's already-advanced feature RNG through the
same-stream bridge, with no copying or reseeding. Tests compare the live
`simplex::WorldgenRandom` and isolated `random::WorldgenRandom` backends after
advancing the stream and across multiple tree calls. Missing neighbours load
actual terrain through the absolute-coordinate, copy-on-write region cache.
Native **UP/FULL support** was verified for **29,873 states at three positions =
89,619 queries**, with `getBlockEntity` returning null in the reference context.
This independently checks the support table used by the adapter.

Postprocessing requests are recorded in their owning chunks. Native flag **16**
(`UPDATE_KNOWN_SHAPE`) suppresses the implicit `getPostProcessPos` query when set;
explicit postprocessing requests remain recorded. The tests verify flags and mark
ownership, not general neighbour updates or ticks. The current finalization
consumer handles the supported mineshaft fence/wall-torch updates.

`crates/bcore-worldgen/data/fallen_tree_configs_26_1.json` records configuration
provenance and the `region_properties` source/support-reference hashes, UP/FULL
query context and full-state postprocessing mappings.

Global region-vegetation hookup still requires the native full-source scheduler;
adapter parity does not establish global placement order.

### Structure candidates and RNG precision

The village candidate helper used SplitMix64 modulo offsets, not vanilla's
LegacyRandomSource. It now delegates to `structure/placement.rs`, which implements
linear/triangular random spread and all four frequency-reduction methods.
Negative region coordinates use floor division; seed arithmetic wraps as in Java.
Mineshafts use spacing 1 and the `legacy_type_3` frequency gate (0.004), including
the separate `setLargeFeatureSeed` stream.

`scripts/StructureReference.java` invokes native `getPotentialStructureChunk` and
`applyAdditionalChunkRestrictions`. The fixture covers eight actual structure-set
configurations (villages, mineshafts, ancient cities, trial chambers, monuments,
mansions, outposts and buried treasure) plus one synthetic default-frequency
configuration. All **54 grids / 228,150 queries** match coordinates and frequency
decisions across six seeds, including negative and extreme signed seeds.
These are candidate checks, **not generated structures**. Biome/terrain admission,
outpost exclusion zones, other structures' weighted selection, pieces and block
entities are not covered by this candidate fixture. Mineshaft integration has
separate admission, piece and region fixtures described above.

JAR inspection also exposed lossy `f32` rounding in the noise and ore RNG paths.
Both `nextDouble` implementations now preserve all 53 bits. The independent
`random_26_1.json` fixture covers 18 native sequences of 2,048 doubles, hashing
their IEEE-754 bits and checking the following long. Rust checks both existing
Xoroshiro/WorldgenRandom implementations against those same sequences. The new
test failed against the old noise implementation and passes after the fix.

The precision fix changes the three world samples by **+1, 0, 0** exact states;
terrain heights, quart biomes and lowest-log proxies remain as shown above. This
small sample delta does not measure the importance or completeness of RNG parity.
Verification at that stage: **101 passed / 2 ignored** across the worldgen suite; both new
fixtures reproduced with `--verify`; workspace release build passed. At that stage,
dead-code warnings were still present in aquifer, density and protocol code.

The full objective and remaining acceptance checks are tracked in
[worldgen-parity-plan.md](worldgen-parity-plan.md).

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

1. Persist and restore the live holder graph, stage claims and unreturned neighbour
   effects. Add saved-neighbour hydration and bounded eviction to the implemented
   shared generation context and `.bcc` column snapshots.
2. Capture native multi-task execution and compare complete regions under matched
   request histories. The live source driver already runs standing/fallen trees,
   ores and supported features; fixed-plan fixtures and the static dependency
   graph do not establish native global source order.
3. Complete the source operations named by coverage and contextual edge effects;
   integrate tick containers with runtime time, ownership and callbacks. The
   resource-key biome bridge is verified, but 4,608 matching quart cells do not
   establish full biome-selection parity on other seeds or all R-tree ties.
4. Extend the real overworld carver/surface resolver to custom heights, retrogen,
   nonempty blending and complete mixed-feature regions. Complete remaining
   structures, including runtime ancient-city admission, references, terrain
   adaptation and generated-data handoff beyond the verified jigsaw component.
5. Generated-entity gameplay: minecart motion, loot unpacking/inventory interaction,
   persistent mutable-entity state and spawner simulation.
6. Additional seeds, ocean/mountain/biome-boundary samples, Nether and End.
7. Optimize measured hot paths after each exactness check; current performance
   is still far from the project goal.

The river hypothesis at `(1000,0)` was disproved: surface cells are forest; the
full-height sample contains forest and deep_dark, with no river.

## Reproduce

Run from the repository root with `BCORE_DATAPACK` unset. The Rust fixture checks
use bundled data; Java is needed only to recapture the native reference.

### Numerical and whole-chunk checks

```powershell
cargo run --release --locked -p bcore-worldgen --example math_lab -- check crates/bcore-worldgen/data/numeric_26_1.json
cargo test --release --locked -p bcore-worldgen --test numeric_reference
cargo build --release --locked -p bcore-worldgen --example dump_chunk
# Requires vanilla 26.1 on port 25571 and the existing opped bot account:
python scripts/worldgen_snapshot.py --capture --output target/parity-current.json
# Iterate against the same saved vanilla data:
python scripts/worldgen_snapshot.py --output target/parity-current.json
# Offline, bundled regression test (no server/datapack directory required):
cargo test --release --locked -p bcore-worldgen --test vanilla_reference
```

### Native component fixtures

The capture commands require the pinned 26.1 JAR and its libraries under
`target/vanilla-775`, `javac` 21+ and a Java 25+ runtime. Adjust `--java`, `--javac`
or `--vanilla` for the local installation. `--verify` compares source/JAR hashes
as well as all samples; a changed probe requires a matching fresh capture.

The shared capture runner accepts **`--build-dir`** to isolate Java compilation
and runtime scratch files. Use a unique directory **and output file for each
concurrent invocation**, even when running the same probe twice. Without the
option, all probes share `target/tree-reference`, suitable for sequential work.

```powershell
# Recreate the isolated tree fixture (JDK 21+ javac and a Java 25+ runtime):
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --build-dir target/native-probes/tree --output target/tree-reference.json --verify crates/bcore-worldgen/data/trees_26_1.json
cargo test --release --locked -p bcore-worldgen --lib isolated_trees_match_vanilla_26_1
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe heightmap --build-dir target/native-probes/heightmap --output target/heightmaps_26_1.json --verify crates/bcore-worldgen/data/heightmaps_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe structure --build-dir target/native-probes/structure --output target/structure-reference.json --verify crates/bcore-worldgen/data/structures_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe random --build-dir target/native-probes/random --output target/random-reference.json --verify crates/bcore-worldgen/data/random_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe ore --build-dir target/native-probes/ore --output target/ore-reference.json --verify crates/bcore-worldgen/data/ores_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe ore_placement --build-dir target/native-probes/ore-placement --output target/ore-placement-reference.json --verify crates/bcore-worldgen/data/ore_placements_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe biome_zoom --build-dir target/native-probes/biome-zoom --output target/biome-zoom-reference.json --verify crates/bcore-worldgen/data/biome_zoom_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe feature_order --build-dir target/native-probes/feature-order --output target/feature-order-reference.json --verify crates/bcore-worldgen/data/feature_order_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe aquifer --build-dir target/native-probes/aquifer --output target/aquifer-reference.json --verify crates/bcore-worldgen/data/aquifers_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe monster_room --build-dir target/native-probes/monster-room --output target/monster-room-reference.json --verify crates/bcore-worldgen/data/monster_rooms_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe mineshaft --build-dir target/native-probes/mineshaft --output target/mineshaft-reference.json --verify crates/bcore-worldgen/data/mineshafts_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe mineshaft_blocks --build-dir target/native-probes/mineshaft-blocks --output target/mineshaft-blocks-reference.json --verify crates/bcore-worldgen/data/mineshaft_blocks_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe mineshaft_start --build-dir target/native-probes/mineshaft-start --output target/mineshaft-start-reference.json --verify crates/bcore-worldgen/data/mineshaft_starts_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe mineshaft_region --build-dir target/native-probes/mineshaft-region --output target/mineshaft-region-reference.json --verify crates/bcore-worldgen/data/mineshaft_regions_26_1.json
cargo test --release --locked -p bcore-worldgen --lib structure::mineshaft
cargo test --release --locked -p bcore-worldgen --test mineshaft_blocks_reference
```

### Mth, carver and entity checks

These run the shared lookup, real carver surface resolver and world-specific
streaming targets. Carver verification includes the historical scripted cases and
the newer evaluated-surface/native-aquifer cases.

```powershell
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe mth --build-dir target/native-probes/mth --output target/mth-reference.json --verify crates/bcore-worldgen/data/mth_26_1.json
cargo test --release --locked -p bcore-worldgen --test mth_reference
cargo test --release --locked -p bcore-worldgen --test ore_reference
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe carver --build-dir target/native-probes/carver --output target/carver-reference.json --verify crates/bcore-worldgen/data/carvers_26_1.json
cargo test --release --locked -p bcore-worldgen --lib carver::
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe entity_packet --build-dir target/native-probes/entity-packet --output target/entity-packets-reference.json --verify crates/bcore-protocol/data/entity_packets_26_1.json
cargo test --release --locked -p bcore-protocol --test entity_packets_reference
cargo test --release --locked -p bcore-protocol --lib world_state::queue_tests -- --test-threads=4
cargo test --release --locked -p bcore-protocol --lib world:: -- --test-threads=4
```

### Standing, fallen and bounded vegetation checks

`vegetation` selects the archived 504-case driver, `standing` selects the 438 full
cases, and `standing_blocks` selects the state table. The updated runner controls
the probe environment internally. Keep build directories distinct because the
archived and full probes share a Java class name. The Rust test targets are already
present.

```powershell
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe vegetation --build-dir target/native-probes/vegetation-bounded --output target/vegetation-bounded-reference.json --verify crates/bcore-worldgen/data/vegetation_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe standing --build-dir target/native-probes/standing --output target/standing-reference.json --verify crates/bcore-worldgen/data/standing_trees_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe standing_blocks --build-dir target/native-probes/standing-blocks --output target/standing-blocks-reference.json --verify crates/bcore-worldgen/data/standing_blocks_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe fallen --build-dir target/native-probes/fallen --output target/fallen-reference.json --verify crates/bcore-worldgen/data/fallen_trees_26_1.json
cargo test --release --locked -p bcore-worldgen --lib region::standing_tests
cargo test --release --locked -p bcore-worldgen --lib tree::
cargo test --release --locked -p bcore-worldgen --lib decoration::
cargo test --release --locked -p bcore-worldgen --lib region::fallen_tests
cargo test --release --locked -p bcore-worldgen --lib region::vegetation_tests
cargo test --release --locked -p bcore-worldgen --test fallen_tree_reference
```

These are fixture/fixed-plan checks. Owning-chunk effect transfer and persistence
are implemented; global region-vegetation scheduling and runtime tick execution
remain pending. For the new `beehive` and `tick` probes and their Rust checks, see
[tree-effects.md](tree-effects.md#verification-and-reproduction). The
[`feature_dependency` probe](feature-scheduling.md#provenance-and-reproduction)
verifies the native graph metadata independently.

### Regression suites

The **2026-10-02 full workspace run completed with exit code 0: 514 passed,
0 failed, 4 ignored**, in **1169.156 s**, including compilation. It used Windows,
four Rayon workers, two Cargo build jobs and four test threads:

```powershell
$env:RAYON_NUM_THREADS = '4'
cargo test --workspace --release --locked --no-fail-fast -j 2 -- --test-threads=4
```

| Package | Passed | Failed | Ignored |
|---|---:|---:|---:|
| bcore-worldgen | 271 | 0 | 3 |
| bcore-protocol | 234 | 0 | 0 |
| bcore-core | 6 | 0 | 0 |
| bcore-plugin | 3 | 0 | 0 |
| bcore-plugin-java | 0 | 0 | 1 |

The existing ignored checks are two worldgen benchmarks, the larger externally
captured `parity_fixture_matches_bcore` suite, and the JVM/plugin-jar end-to-end
test. They are not passing coverage. All selected unit/integration/doc-test
targets completed, including **162 protocol library tests (87.47 s)**,
**15 persistence tests (215.35 s)** and **11 terrain-encoding tests (500.30 s)**.
Persistence includes the 7×7 round trip; terrain encoding includes all 64 dispersed
fluid-test chunks. The worldgen library passed **206 tests / 2 ignored**;
the package total adds its integration suites, including the fixed tree/jigsaw
regressions and native numerical/surface/cave/sculk references.

Saved command, status and full output:
`target/generation-checks-20261001/tests-1790908713123380300.{json,log}`.
The earlier two-worker attempt reached its external **1200 s** deadline during
terrain encoding; it was not counted as a completed pass. The successful runner
allowed **3600 s**. Workspace release build, `cargo fmt --all --check`, report
JSON/binary/capture-hash validation and `git diff --check` also passed.

#### Earlier checkpoints

These commands run the current sources. The **123 / 2** worldgen result and
**178-pass** protocol breakdown above belong to the earlier numerical/pre-streaming
baseline. Run the slow protocol targets separately and allow an outer timeout
longer than the recorded **595.40 s** terrain run (for example, 900 s). Record each
completed test summary before reporting an aggregate.

The saved `target/worldgen-merged-check.log` records the **163 passed / 2 ignored**
checkpoint. `target/worldgen-tree-effects-final.log` records **182 passed /
2 ignored**, including eight tick-container tests and native hive rewrite checks.
The follow-up passed the **28 region, 8 tick-container,
18 storage and 21 protocol integration tests** listed in
[tree-effects.md](tree-effects.md#verification-and-reproduction). These targeted
results are not an additional full-suite aggregate.

For protocol, `target/protocol-merged-check.log` completed with a failing
`chat_flow::help_and_list_reply_on_system_chat` test. The later
`target/protocol-final.log` records **160 library, 12 chat, 5 flat-format,
3 vanilla-chunk and 15 persistence passes**, including that chat test, but ends
during `chunk_terrain` without its result. It does not establish a completed
all-green run. The subsequent v4 storage/integration checks passed; the complete
234-test protocol result is recorded above. The earlier published commit `4d8569f`
completed GitHub CI successfully (run `36428955481`); that is the pre-v4 baseline.

The earlier merged workspace release build and formatting checks passed. Its quality scan
reported no errors or auto-fixable findings after cleanup; 19 non-auto-fixable
warnings remain, primarily existing type/function complexity, large modules and
plugin API documentation. No lint rules were disabled.

Earlier **13 carver (0.25 s)**, **45 tree**, **six queue** and **26 world** test
results retain their checkpoint labels, as do the Mth/fallen-adapter measurements.
The whole-chunk table remains tied to the executable hash recorded above.

```powershell
cargo test --release --locked -p bcore-worldgen --no-fail-fast -- --test-threads=4
cargo test --release --locked -p bcore-protocol --lib -- --test-threads=4
cargo test --release --locked -p bcore-protocol --test chunk_format
cargo test --release --locked -p bcore-protocol --test chunk_persistence -- --test-threads=4
cargo test --release --locked -p bcore-protocol --test chunk_terrain -- --test-threads=4
cargo test --release --locked -p bcore-protocol --test dungeon_block_entities
cargo test --release --locked -p bcore-protocol --test mineshaft_entities
cargo test --release --locked -p bcore-protocol --test tree_effects
cargo test --release --locked -p bcore-protocol --test queued_entity_delivery
```

The native packet and targeted lifecycle checks are also documented in
[entity-streaming.md](entity-streaming.md#reproduce).

`scripts/bundle_worldgen.py` regenerates the embedded bundle from the pinned 26.1
JAR, native climate capture and BCore wire/save registry map. `BCORE_DATAPACK` is an explicit override;
missing override files are errors. Old saved worlds retain old chunks: compare
fresh generator output or use a separate new world directory for gameplay tests.
