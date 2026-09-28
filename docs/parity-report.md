# BCore / vanilla 26.1 parity

**Worldgen parity is incomplete.** Component checks and the whole-chunk snapshot
below have different coverage; neither establishes all-world/all-dimension parity.

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
  streaming run at that point emitted **no compile warnings**. Final post-change
  full-suite totals are pending.

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
from unchanged `.bcc` v3 generated data, independently of placement RNG. This
contract is for immutable **GENERATED / pre-gameplay** records; persistent mutable
entity identity, motion and inventory state remain future work.

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
26 tests**, including **four new adoption regressions**; final post-change suite
totals still await a complete run.

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
the newest combined protocol run remains pending.

## Whole-chunk snapshot

Re-run on 2026-09-25 after the numerical and mineshaft changes. The three exact
state counts remain unchanged; `parity-results.json` records the new executable
hash. This is an integration check on these captures, not evidence that numerical
rounding errors are harmless on other coordinates/seeds. Later targeted entity,
fallen-tree and carver checks do not replace this executable-specific snapshot.
Final integrated suite totals and a snapshot after the latest queue, surface and
standing-tree changes **await confirmation**; the following counts and
hash remain the earlier numerical/mineshaft snapshot.

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
  `14efc60844e9bfe76b4472d91791f70c7686bf1df991d465b989ac39f653b9d9`.

## Results

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

### Latest underground-generation changes

The ore region and aquifer corrections add **964, 299, 1770** matching states
relative to the preceding double-precision baseline: **+3033 total**. Of these,
the aquifer correction adds **836** in the third region. The current comparison
still has **8265 differing states**, including vegetation, cave features and
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
The `.bcc` writer now uses format version **3** and the reader accepts versions
**1, 2 and 3**. Version 3 also stores generated minecarts and structure metadata.
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

The targeted tree result is **45 passed**: 5 standing-region, 18 tree,
4 decoration, 5 fallen-region, 4 bounded-vegetation-region and 9 fallen-reference
tests. The older **37 standing**, **655 fallen** and **504 bounded placement-driver
cases** remain preserved. The historical driver is archived at
`scripts/vegetation-bounded/VegetationReference.java`; the root
`scripts/VegetationReference.java` now probes full standing trees/selectors.

Global region vegetation is **not hooked up**. Hive NBT and tick requests remain
region-owned in `TreeEffects`; `finish_chunk` asserts that they must be transferred
before finalization. Persistence, tick execution and native full-source scheduling
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

1. A real staged generation region for cross-chunk tree/ore reads and writes.
   Region-aware standing/fallen kernels and fixed-plan replays are implemented;
   the documented ChunkPyramid is not a functioning native dependency scheduler.
2. Hook region vegetation into that schedule, transfer/persist `TreeEffects`, and
   complete remaining jungle variants, contextual edge effects and tick execution.

   Fallen trees pass **655 native cases** both in isolation and through the live
   `FeatureRegion` adapter. Global region vegetation remains unhooked.
   An earlier partial experiment omitted decorators and shared-region reads and
   changed sample matches by `-1, 0, 0`; that experiment was removed.
3. Complete global feature admission and verify R-tree equal-distance behavior.
   Region-aware tree selectors already use biome zoom. Matching these
   4,608 cells does not establish full biome-selection parity on other seeds.
4. Extend the real overworld carver/surface resolver to custom heights, retrogen,
   nonempty blending and complete mixed-feature regions. Complete remaining
   structures, including ancient cities, which explain some deep-dark differences
   at the origin.
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

These are fixture/fixed-plan checks. Global region-vegetation scheduling and
`TreeEffects` persistence are not enabled by running them.

### Regression suites

These commands run the current sources. The **123 / 2** worldgen result and
**178-pass** protocol breakdown above belong to the earlier numerical/pre-streaming
baseline. Run the slow protocol targets separately and allow an outer timeout
longer than the measured 462 s terrain run (for example, 900 s). Record each
completed test summary before reporting an aggregate.

Newest confirmed component results: **13 carver tests (0.25 s)**, the **45-test
tree group** detailed above, **six queue tests**, **26 protocol world tests**, and
the 27-sample native packet reference. The fresh worldgen suite passed **163 tests
with 2 ignored** and the fresh snapshot is recorded above. Earlier Mth/fallen-adapter
totals retain their stage labels; the complete post-change protocol suite remains
pending.

```powershell
cargo test --release --locked -p bcore-worldgen --no-fail-fast -- --test-threads=4
cargo test --release --locked -p bcore-protocol --lib -- --test-threads=4
cargo test --release --locked -p bcore-protocol --test chunk_format
cargo test --release --locked -p bcore-protocol --test chunk_persistence -- --test-threads=4
cargo test --release --locked -p bcore-protocol --test chunk_terrain -- --test-threads=4
cargo test --release --locked -p bcore-protocol --test dungeon_block_entities
cargo test --release --locked -p bcore-protocol --test mineshaft_entities
```

The native packet and targeted lifecycle checks are also documented in
[entity-streaming.md](entity-streaming.md#reproduce).

`scripts/bundle_worldgen.py` regenerates the embedded bundle from the pinned 26.1
JAR, native climate capture and BCore wire/save registry map. `BCORE_DATAPACK` is an explicit override;
missing override files are errors. Old saved worlds retain old chunks: compare
fresh generator output or use a separate new world directory for gameplay tests.
