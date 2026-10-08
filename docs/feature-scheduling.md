# Native feature scheduling: Java 26.1

The pinned JAR's dependency graph is captured in
[`feature_dependencies_26_1.json`](../crates/bcore-worldgen/data/feature_dependencies_26_1.json).
`GenerationWorld` now uses this graph to retain shared, status-aware chunks across
requests. **A write radius does not specify global source order; matching native
multi-task histories still requires source-execution traces.**

## Oracle and scope

[`FeatureDependencyReference.java`](../scripts/FeatureDependencyReference.java)
calls `SharedConstants.tryDetectVersion` and `Bootstrap.bootStrap`, then reads
the actual `ChunkPyramid.GENERATION_PYRAMID` and `LOADING_PYRAMID` objects.
The class in this JAR is **`ChunkPyramid`**, not `ChunkPyramids`.

The fixture contains:

- **12 ordered statuses**, native indices/parents, chunk types and heightmap sets;
- **24 steps**, with both dependency tables and `blockStateWriteRadius`;
- **156 native `ChunkGenerationTask.getRadiusForLayer` queries**, checked against
  `ChunkStep.getAccumulatedRadiusOf`, including the target's special radius zero;
- **14 diagonal-neighbour table queries** using native chessboard distance,
  including positions outside each table;
- rejection of `getRadiusOf(FEATURES)` on both FEATURES dependency objects in
  both pyramids, and the **11 native decoration-step enum values**.

No status list or dependency radii are embedded in the probe. The radius-only
task uses its native constructor with a position/target and null map/cache; the
queried method only reads the target and pyramids. It never runs a generation
task. Neighbour samples query the dependency tables, not a live WorldGenRegion.
This is an oracle for **graph metadata and scheduler scaffolding**, not terrain,
feature placement, write-guard execution or inter-task order.

## Reading the tables

`by_radius[r]` applies at **exact chessboard distance**
`r = max(abs(dx), abs(dz))`, in chunks. Distance zero is the source itself.
`S@a..b` below means status S in every ring a through b. `STARTS`, `REFERENCES`
and `INIT_LIGHT` abbreviate `STRUCTURE_STARTS`, `STRUCTURE_REFERENCES` and
`INITIALIZE_LIGHT`; the fixture stores full `minecraft:` names.

- **Direct**: requirements of this step. WorldGenRegion also uses this table to
  limit the status a caller may request at each position.
- **Accumulated**: the native builder's inherited requirements, used to plan
  earlier layers and retain holders. This is not the region's direct read window.
- **Write**: WorldGenRegion's block-state write radius: `-1` permits none, `0`
  permits the source chunk, `1` permits its 3×3 square. This says nothing about
  biome/structure metadata writes, lighting, or live tick execution.

An empty dependency list really is empty, although native `getRadius()` returns
zero for it. EMPTY's parent is itself; that does not create a dependency cycle.

### Generation pyramid

Rows are the native status order, indices 0 through 11.

| Target | Direct dependencies | Accumulated dependencies | Write |
|---|---|---|---:|
| EMPTY | — | — | -1 |
| STARTS | EMPTY@0 | EMPTY@0 | -1 |
| REFERENCES | STARTS@0..8 | STARTS@0..8 | -1 |
| BIOMES | REFERENCES@0; STARTS@1..8 | REFERENCES@0; STARTS@1..8 | -1 |
| NOISE | BIOMES@0..1; STARTS@2..8 | BIOMES@0..1; STARTS@2..9 | 0 |
| SURFACE | NOISE@0; BIOMES@1; STARTS@2..8 | NOISE@0; BIOMES@1; STARTS@2..9 | 0 |
| CARVERS | SURFACE@0; STARTS@1..8 | SURFACE@0; BIOMES@1; STARTS@2..9 | 0 |
| FEATURES | CARVERS@0..1; STARTS@2..8 | CARVERS@0..1; BIOMES@2; STARTS@3..10 | 1 |
| INIT_LIGHT | FEATURES@0 | FEATURES@0; CARVERS@1; BIOMES@2; STARTS@3..10 | -1 |
| LIGHT | INIT_LIGHT@0..1 | INIT_LIGHT@0..1; CARVERS@2; BIOMES@3; STARTS@4..11 | -1 |
| SPAWN | LIGHT@0; BIOMES@1 | LIGHT@0; INIT_LIGHT@1; CARVERS@2; BIOMES@3; STARTS@4..11 | -1 |
| FULL | SPAWN@0 | SPAWN@0; INIT_LIGHT@1; CARVERS@2; BIOMES@3; STARTS@4..11 | -1 |

### Loading pyramid

Every loading step has write radius **-1**.

| Target | Direct dependencies | Accumulated dependencies |
|---|---|---|
| EMPTY | — | — |
| STARTS | EMPTY@0 | EMPTY@0 |
| REFERENCES | STARTS@0 | STARTS@0 |
| BIOMES | REFERENCES@0 | REFERENCES@0 |
| NOISE | BIOMES@0 | BIOMES@0 |
| SURFACE | NOISE@0 | NOISE@0 |
| CARVERS | SURFACE@0 | SURFACE@0 |
| FEATURES | CARVERS@0 | CARVERS@0 |
| INIT_LIGHT | FEATURES@0 | FEATURES@0 |
| LIGHT | INIT_LIGHT@0..1 | INIT_LIGHT@0..1 |
| SPAWN | LIGHT@0 | LIGHT@0; INIT_LIGHT@1 |
| FULL | SPAWN@0 | SPAWN@0; INIT_LIGHT@1 |

Bytecode distinguishes loading tasks from generation: loading STARTS invokes
`loadStructureStarts`; REFERENCES through FEATURES and SPAWN are pass-through;
INITIALIZE_LIGHT, LIGHT and FULL retain their native tasks. EMPTY is special in
`ChunkMap.applyStep`, which loads the stored chunk or creates an empty one.
`ChunkGenerationTask` checks persisted statuses before choosing loading, and
chooses generation/loading **per holder and layer**. The smaller loading pyramid
cannot be used to generate missing terrain or skip fresh features.

## Exact neighbourhood requirements

These are native task-layer radii, including earlier statuses implied by a later
dependency and the requested target itself. A radius r covers `(2r + 1)²` chunks.

| Layer to reach | Fresh FEATURES(c) | Fresh FULL(c) | Load-only FULL(c) |
|---|---:|---:|---:|
| EMPTY, STRUCTURE_STARTS | 10 | 11 | 1 |
| STRUCTURE_REFERENCES, BIOMES | 2 | 3 | 1 |
| NOISE, SURFACE, CARVERS | 1 | 2 | 1 |
| FEATURES | 0 | 1 | 1 |
| INITIALIZE_LIGHT | — | 1 | 1 |
| LIGHT, SPAWN, FULL | — | 0 | 0 |

For one fresh feature source, the graph therefore calls for **441** EMPTY/STARTS
holders, **25** reference/biome chunks, **9** noise/surface/carver chunks, and that
source's FEATURES task. The direct FEATURES region spans **289** positions:
its inner nine at CARVERS, the remaining 280 at STARTS. Its accumulated radius
10 is a planning envelope, not permission to read blocks at distance 10.

For a fresh FULL target, the corresponding counts are **529**, **49**, **25**,
**9 FEATURES/INITIALIZE_LIGHT**, and the center's remaining steps. In particular:

- FEATURES(c) itself does **not** require any neighbouring FEATURES task.
  Both direct and accumulated `getRadiusOf(FEATURES)` throw. The step-level
  result zero is a target special case, not a self-dependency.
- FULL(c), through LIGHT, requires FEATURES on c and all eight neighbours.
  These are all possible direct incoming block-state writers under radius 1.
  Finishing FEATURES(c) alone is insufficient to freeze c's decorated blocks.
- Native `ChunkGenerationTask.create` initially retains the **generation**
  pyramid's EMPTY radius even when loading may subsequently suffice. The
  load-only radius column describes layer work, not that initial cache claim.
- These envelopes do not require generating CARVERS for every outer holder.
  A holder at STARTS can contain structure metadata without filled terrain.

FULL is also distinct from ticking preparation: `ChunkMap.prepareTickingChunk`
requests a **3×3 FULL neighbourhood** before calling `postProcessGeneration`.
That consumer is outside these 24 pyramid steps. The separate
[tick-container captures](tree-effects.md#native-tick-container-semantics) cover
chunk-local preparation/save/order semantics; they do not invoke this consumer.

### Terrain, carvers and structures

`ChunkStatusTasks` establishes the order: structure starts → references → biomes
→ noise/aquifers → surface → carvers → features. Structure starts/references
are metadata stages. Structure **block placement** happens inside FEATURES at
the structure's decoration step.

`NoiseBasedChunkGenerator.createNoiseChunk` uses
`Beardifier.forStructuresInChunk`, so terrain adaptation needs the correct
structure metadata before density filling, not a later structure-block overlay.
Blender and below-zero retrogen also have stage-specific paths; their support
requires separate captures beyond a fresh, unblended overworld baseline.

CARVERS has a radius-8 direct neighbourhood but a radius-0 block write window.
`NoiseBasedChunkGenerator.applyCarvers` enumerates **17×17 source chunks**, obtains
each source's `carverBiome` generation settings via a noise-biome supplier, and
carves the **target** after its surface stage. This does not require all 289
sources to have filled terrain, completed CARVERS, or even stored BIOMES. Do not
confuse carver source reach with the radius-1 CARVERS prerequisite of FEATURES.

### One source's FEATURES task

`generateFeatures` primes the four final heightmaps, creates WorldGenRegion,
calls `ChunkGenerator.applyBiomeDecoration`, then generates blending border ticks.
The captured decoration order is:

```text
0 RAW_GENERATION          1 LAKES                  2 LOCAL_MODIFICATIONS
3 UNDERGROUND_STRUCTURES  4 SURFACE_STRUCTURES     5 STRONGHOLDS
6 UNDERGROUND_ORES        7 UNDERGROUND_DECORATION 8 FLUID_SPRINGS
9 VEGETAL_DECORATION     10 TOP_LAYER_MODIFICATION
```

Within each step, native code places structures first, then placed features in
sorted `FeatureSorter` index order. The candidate feature indices are the union
from **all section biome palettes in the source's 3×3 neighbourhood**, intersected
with the generator's possible biomes. Placement modifiers still perform their
own biome checks. Preserve the decoration seed, native structure/feature indices
and each child's shared RNG stream; an unsupported feature must not silently
become a skipped RNG/world operation. The existing `feature_order_26_1.json`
captures within-source feature ordering, not scheduling between sources.

Standing/fallen trees at step 9 need the actual live state left by earlier
structures, lakes, ores, underground decoration, springs, preceding vegetation,
and previously executed neighbouring sources. Heightmaps, support checks,
replacement predicates and RNG branching can all change with that state.

## What the graph does and does not order

The inspected native bytecode adds useful facts beyond the static tables:

1. `ChunkGenerationTask.scheduleLayer` visits increasing X in the outer loop,
   increasing Z in the inner loop **for one task's layer**. `runUntilWait` waits
   for that layer before advancing. `GenerationChunkHolder` claims each status
   once and reuses its future when another task reaches the same holder.
2. `ChunkMap` runs worldgen continuations through a `ConsecutiveExecutor` and
   `ChunkTaskDispatcher`. Its priority queue uses per-priority linked maps;
   `pop` takes the first queued key, not the least coordinate. Task submissions,
   priority changes, completed work and asynchronous resumptions affect which
   continuation runs next. An absent same-status dependency is not permission
   to parallelize overlapping feature writes without coordination.
3. `WorldGenRegion.getChunk` limits the **requested** status using the direct
   table and returns the holder's chunk at that dependency future. This is a
   shared mutable chunk, not an immutable snapshot of that status. Earlier
   feature writes can already be visible even when the requested status is
   CARVERS. Out-of-table or too-high-status requests fail; lazily supplying
   arbitrary fully carved chunks changes the native contract.
4. `ensureCanWrite` compares the destination chunk to the **current source** and
   its write radius. Normal vertical block bounds and upgrading-height checks
   need their own handling. Marks, ticks and entity storage use additional
   paths; the block-state radius is not a complete side-effect policy.

Consequently, **there is no captured evidence of a deterministic global X/Z
FEATURES order or of request-order-independent decorated output**. The local
loop order does not establish either. This pass measured no native multi-task
execution or request-order output differences.

A source two chunks away cannot directly write the target under radius 1, but
can write its neighbour, which may later read that change while generating a
tree into the target. Shared structure-piece state adds another interaction.
The finite static footprint does not prove that target-local replay from fresh
terrain reproduces the history of an already partially generated world.

## Current BCore runtime

- `GenerationWorld` owns the seed, live `FeatureRegion`, structure starts,
  per-holder progress and source sequence numbers. A world-scoped mutex coalesces
  status claims and serializes feature writes. Repeated and adjacent requests
  reuse the same holders; constructing another world creates a separate history,
  including when its seed matches.
- Planning uses the accumulated pyramid, while region reads and writes use the
  current source's direct dependencies and write radius. Missing dependencies
  fail explicitly. Live generation does not fetch pristine chunks from the old
  immutable terrain cache to replace already-written neighbours.
- `generation/driver.rs` collects the 3×3 biome-palette union and visits native
  decoration steps and placed-feature indices. Supported structures, ores,
  dungeons, trees, base features, cave features and sculk operate on that region.
  Placement and cave bodies share the same Gaussian cache across feature reseeding.
  Admission maps BCore biome IDs to the native catalog by resource key; the
  independent wire/save registry is not used as a native numeric index.
- Retained starts/references now cover mineshafts, villages, ancient cities, trail
  ruins, trial chambers, buried treasure, swamp huts, jungle and desert pyramids. All
  structure types share the source RNG and use their native structure indices,
  before the separate placed-feature indices. Scattered piece flags and cached
  reference boxes survive movement, clipping and `.bcc` round trips. Template and
  hut entity requests remain explicit until their factories/finalizers execute.
- Fossils use the native templates/processors and continuing feature RNG, including
  shape completion between passes. Desert pyramids additionally consume the single
  source-region RNG and separate positional archaeology streams.
- BIOMES fills ascending sections in native X/Y/Z query order, independently of
  Y/Z/X palette storage. Each world's climate search keeps its own tie history.
- INITIALIZE_LIGHT and LIGHT execute against shared native-style storage. Later
  source writes feed ordered incremental updates into initialized columns, and
  publication preserves lazy-zero, materialized-zero and absent layers. Packets
  and `.bcc` retain these distinctions.
- NOISE uses a density-cache scope per Rayon job. Each job keeps one graph and
  complete evaluation context, reuses interpolation corners across its columns,
  and clears worker caches on entry, exit and unwind. Separate worlds and changed
  geometry/blending contexts cannot reuse that job's entries.
- Successful writes retain owning-chunk block entities, entities, hive occupants,
  duplicate postprocessing marks and raw tick requests. Effect transfer also runs
  after a failed feature; failures retain their state and are not retried as fresh
  sources. The earlier `RegionPlan` and target-local helpers remain useful to
  isolated fixtures but are not the server's live generation context.
- Proto block-entity writes install pending typed `DUMMY` tags. Explicit lookups
  materialize those tags before loot/occupant callbacks; NBT-less template writes
  do not eagerly create defaults. The v5 store preserves this pending/live boundary.
- SPAWN uses retained biome/block/light storage, Legacy decoration RNG, a separate
  region stream and independent entity entropy. Accepted saves are retained before
  later callbacks run; partial/failing attempts are not retried as fresh spawns.
- Server `World::chunk`, `World::generate`, queued jobs and world clones use one
  `GenerationWorld`. `World::try_generate` returns both a `ChunkColumn` and its
  `GenerationCoverage`; `generation_progress` exposes live claims. Persistence
  retains the returned column's generated data, including typed sculk NBT and
  unconsumed marks, as described in [tree-effects.md](tree-effects.md).
- `dump_chunk` retains one context for its complete coordinate list. Its NDJSON
  includes `generation_coverage`, source sequence numbers, missing work, generated
  entities, typed sculk data, ticks and marks. `worldgen_snapshot.py` deduplicates
  overlapping sources and rejects inconsistent reports or reset sequence numbers.

### Coverage is distinct from native completion

The FULL request currently executes supported work through SPAWN in the native
FULL dependency envelope. `Partial` means a finished attempt with named omissions;
it is not a completed native status. FULL remains pending. SPAWN's server-input
API is implemented; live clock/settings updates still require hookup. Structure
density adaptation and noise ore-vein filling are implemented; missing structure
families and unsupported callbacks still appear in coverage.

WG maps freeze after CARVERS and remain available to subsequent features. Native
FULL conversion removes them. BCore retains them until that conversion is actually
implemented, so their presence still differs on FULL requests. FULL-request
snapshots explicitly materialize the target's block entities, but that BE-only
boundary does not complete FULL. Pending `DUMMY` tags remain observable beforehand.
The LIGHT-history's final read follows an implicit native FULL completion, exposing
36 BE fields until BCore implements the real ticket-driven lifecycle transition.

`incoming_sources_finished` means all possible direct incoming writers have finished
their supported attempts. It does not mean all features, lighting or native ticking
preparation succeeded. Query `coverage.is_complete()` and the missing-work lists.
The server reports partial coverage once per world and exposes details through
`try_generate`; diagnostic output retains those details on every requested chunk.

The store currently saves column snapshots, not the live holder graph. A loaded-only
column has no live progress entry. Hydration of saved neighbours, persisted claims,
cross-restart ownership of unreturned neighbours and bounded holder eviction still
need integration. The loading pyramid metadata is not evidence of that runtime path.

## Remaining integration and validation

1. **Persistent holder ownership.** Save and restore generation status and outgoing
   neighbour effects alongside columns. Distinguish opaque legacy snapshots from
   fresh work; never replay a stored source or replace player edits with fresh
   terrain. Add safe holder eviction backed by this persistence path.
2. **Native schedule coverage.** The actual-server oracle instruments
   ChunkMap/ChunkGenerationTask/GenerationChunkHolder execution and has matched
   fresh, adjacent, reversed and bootstrap request histories. Extend it to reloads,
   broader structure/biome combinations and multiple workers. Record request
   target, ticket/queue priority, stage entry/completion, generation versus
   loading selection, skipped/already-complete work, source FEATURES entry/exit
   and structure-start identity. Use sequence IDs and observed happens-before
   edges rather than timestamps alone. Feature work is currently serialized;
   parallelize only stages/conflicts justified by native evidence.
3. **Remaining terrain and structure coverage.** Extend the integrated starts,
   references, terrain adaptation, template placement and metadata to the remaining
   families, including strongholds, monuments, mansions, ocean ruins and portals.
   Compare per-stage states, biomes, heights and carving masks. Component fixtures
   and graph metadata do not by themselves prove the integrated result.
4. **Complete feature coverage.** Resolve the named missing operations in
   `GenerationCoverage`, preserving caller-owned RNG and live-world observations.
   Do not treat an unsupported kernel as a successful no-op because one sampled
   origin happened to fail placement.
5. **Finalization and validation.** Wait for the appropriate incoming-source
   FEATURES closure before publishing/finalizing a target; distinguish FEATURES,
   FULL and native ticking preparation/postprocessing snapshots. Integrate the
   tested tick-container helpers with runtime ownership, time and callback
   execution, then compare native and BCore multi-chunk results under matched
   request histories and worker counts.

Required integration captures include positive/negative coordinates, biome and
chunk corners, crossing standing/fallen trees, hive/leaf-edge effects, dungeons,
ore air-exposure interactions, and overlapping structures with mutable piece
state. Capture per-source preconditions and outgoing writes/effects as well as
the final target. Compare single-target, adjacent/reversed/spiral requests,
already-decorated neighbours, save/reload and multiple worker counts. If native
histories differ, preserve/report that distinction rather than asserting a
canonical order from a fixed-plan fixture. The live hookup needs these captures
and remaining feature implementations before it can establish native parity.

## Provenance and reproduction

- JAR: `target/vanilla-775/versions/26.1/server-26.1.jar`
- JAR SHA-256: `a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`
- Compiler used: `javac 21.0.11`
- Runtime used: Temurin **25.0.4.1+1**, at
  `target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe`
- Classpath: isolated build directory, pinned JAR, then sorted recursive
  `target/vanilla-775/libraries/**/*.jar`.
- `probe_sha256` hashes the raw concatenation, in this order:
  **`[scripts/TreeReference.java, scripts/FeatureDependencyReference.java]`**.
  No other helper is required. Captured hash:
  `876289544df8743b6dcea98ab9dc5052a3f17670502c2a731206f82810deec46`.
- Payload marker: **`FEATURE_DEPENDENCY_REFERENCE=`**. Bootstrap redirects
  stdout through logging, so locate the marker within the line.

The shared runner now exposes **`--probe feature_dependency`**. From the
repository root, with the pinned JAR/libraries, `javac` 21+ and Java 25+ available:

```powershell
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe feature_dependency --build-dir target/native-probes/feature-dependency --output target/feature-dependencies-reference.json --verify crates/bcore-worldgen/data/feature_dependencies_26_1.json
```

This compiles the two sources and compares the entire parsed fixture, including
provenance. Independent recapture with `--verify` passed on **2026-09-29**. Use a
unique build directory and output path for each concurrent invocation; adjust
`--java`, `--javac` or `--vanilla` for the local installation.

Bytecode research can be repeated with JDK 21's `javap` against this same JAR:

```powershell
javap -classpath "target/vanilla-775/versions/26.1/server-26.1.jar" -c -p `
  net.minecraft.world.level.chunk.status.ChunkPyramid `
  'net.minecraft.world.level.chunk.status.ChunkStep$Builder' `
  net.minecraft.world.level.chunk.status.ChunkDependencies `
  net.minecraft.world.level.chunk.status.ChunkStep `
  net.minecraft.world.level.chunk.status.ChunkStatusTasks `
  net.minecraft.server.level.WorldGenRegion `
  net.minecraft.server.level.ChunkGenerationTask `
  net.minecraft.server.level.GenerationChunkHolder `
  net.minecraft.server.level.ChunkMap `
  net.minecraft.server.level.ChunkTaskDispatcher `
  net.minecraft.server.level.ChunkTaskPriorityQueue `
  net.minecraft.world.level.chunk.ChunkGenerator `
  net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator
```

Use `javap -v -p` on `ChunkPyramid` and `ChunkStep$Builder` to resolve lambda
bootstrap method handles to `ChunkStatusTasks` names. The tables above come
from bootstrapped objects, not from transcribing those builder constants.
