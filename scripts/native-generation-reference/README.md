# Actual 26.1 generation histories

This oracle runs the pinned vanilla server's **real `ServerChunkCache`,
`ChunkMap`, `ChunkGenerationTask`, `GenerationChunkHolder`, `ChunkStep` and
configured/placed features**. It records execution and complete chunk data, not
just the status pyramid. The Java code is reflection-based so `javac 21` can
compile it; execution requires the repository's Java 25 runtime.

## Capture

From the repository root:

```powershell
python scripts/native-generation-reference/capture.py --output target/native-history/my-new-run --request "0,0,biomes" --request "0,0,noise" --request "0,0,surface" --request "0,0,carvers" --request "0,0,features" --request "0,0,full"
python scripts/native-generation-reference/oracle.py verify target/native-history/my-new-run
```

- Default seed: **846692123413862008**. Use `--seed 0`, `--seed 42`, etc.
- Negative coordinates: `--request=-1,-1,features` (avoids argparse treating the
  leading minus as an option).
- Requests execute synchronously in the order supplied. Repeated coordinates
  use the same live native server and holders.
- Default `--bootstrap before_spawn` enters the harness at the start of native
  `MinecraftServer.setInitialSpawn`. The real server/registries/overworld/cache
  have been initialized; the trace records the actual existing holder count.
  This is a deliberately chosen fresh-world boundary.
- `--bootstrap native` lets the original `loadLevel`, initial-spawn search and
  preparation finish. Their native chunk requests and stages are part of the
  trace, before the scripted requests. It then enters the same runner.
- Both modes end at the final request return. An entered `MinecraftServer`
  gameplay tick or `ServerLevel.tick` fails the capture. Game time and tick
  counts are recorded. Native SPAWN and FULL stages still run when requested.
- Each output must be a **new directory under this repository's `target/`**.
  Its fresh server uses loopback port **0** (OS-assigned), no RCON/query/management
  service, and its own world/build/log files. Existing output directories are
  rejected. Native worlds and historical Mineflayer snapshots are independent.

The runner verifies native JAR SHA256
`a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52` at
`target/vanilla-775/versions/26.1/server-26.1.jar` and uses
`target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe`. ASM 9.8 is downloaded into the
isolated build directory from Maven Central, checked against the repository
digest, and included in provenance with its SHA256.

## Trace and snapshot contract

`events.jsonl` contains an increasing **observation sequence**, request/phase and
thread, with explicit stage IDs and FEATURES source sequence IDs. Events include:

- Native request calls, holder claims/completion, task creation/layer scheduling,
  per-chunk generation/loading choice, waits and dispatcher submission/execution.
- Actual `ChunkStep.apply` entry and future completion. EMPTY's native
  load/create path is recorded separately at `ChunkMap.applyStep` completion.
- **Before and after** complete source chunk snapshots for every executed stage.
  FEATURES snapshots include **every chunk of the radius-8 direct cache** (289),
  since structures can retain mutable pieces outside the 3×3 block-write square.
- Placed-feature entry/exit, structure/feature labels, native RNG counters,
  exact Xoroshiro state, and cached Gaussian flags/raw double bits. Version 4
  also records the next native `nextLong` from a **separate native RNG copy**;
  the live feature RNG is never advanced for measurement.
- Accepted/rejected native write guards, block writes and flags, raw worldgen
  tick requests with sub-tick order, postprocessing calls and generated entities.
  Direct section writes (e.g. bulk ore placement) are represented in complete
  stage snapshots; they are not all `WorldGenRegion.setBlock` callback events.
- Actual native heightmap queries. For forest grass/seagrass WG queries, an
  additional read-only scan applies the native heightmap predicate to native
  states. It does not call another height getter or prime a heightmap.

Each snapshot references content-addressed `.nbt.gz` and `.json.gz` blobs. The
hash is over **uncompressed bytes**. NBT is produced by native
`SerializableChunkData.copyOf(...).write()` and `NbtIo.write` and preserves:
all block states/properties, quart biomes, stored heightmaps, structures,
block entities, proto entities, block/fluid ticks, postprocessing, carving mask,
blending/retrogen/upgrade data, native light data and generation status.

The extra blob records **all live heightmap arrays**, including retained WG
maps excluded by the native save-status filter, native section counters,
direct `LevelChunkSection.getBlockState` samples to independently check the
NBT decoder, and chunk-owned loaded level entities after FULL. Native starts
also have identity and observed iteration-order records. Raw blobs are the
typed/float-bit authority; the Python decoder is a diagnostic view.

Instrumentation wraps native stage futures with an **observer completion
barrier** so post-stage snapshots finish before the holder can expose that
future to later work. It affects execution timing. It does not replace the
native queues, dependencies, stage bodies or feature algorithms. Version 2+
uses `==`-based identity bookkeeping, avoiding extra `identityHashCode` calls.
Observation IDs and dispatcher interleaving are not expected to be byte-for-byte
stable across processes; compare matched requests, actual source order, chunks
and effects. All terminal stage entry/exit pairs are validated.

## Replay BCore with the same requests

On the shared validation host, submit heavy commands through the serialized
runner. It supplies `CARGO_TARGET_DIR` and retains job status and full logs in
`target/generation-checks/`:

```powershell
python scripts/run_generation_check.py --background native-history-fresh -- python -B scripts/native-generation-reference/replay_bcore.py target/native-history/my-new-run --output target/native-history/my-new-bcore-run --timeout 3600
```

This tool snapshots the actual production source/data read-only into the new
target directory, archives it, and builds the diagnostic against that frozen
copy using **`cargo build --release -j 1 --locked`**. This avoids mixed-revision
builds while other workers edit their own runtime files. One
`Arc<GenerationWorld>` and one production `NativeChunkRequests` runtime are
retained for the entire history. Each request uses `get_chunk(pos, status,
create)` and retains the actual `ReturnedChunk`. The cache, conditional owner
thread pumping and background generation therefore use the production request
route.

### Schema 3: returned objects and holder watches

The plan, every JSONL row and the executable's `--version` response advertise
`schema: 3` and `observation_contract: native_get_chunk_returned_object_v3`.
History build provenance additionally records `replay_schema: 3` and
`snapshot_boundary_version: 3`. Schema 1/2 history executables and JSONL outputs
are rejected; create a fresh build for this contract.

- `seed` and `game_time` are exact signed-i64 strings. This fresh-world contract
  requires recorded **zero** game time and zero gameplay ticks. The plan checks
  the observed bootstrap seed against the exact config seed.
- The plan and rows carry `native_provenance_sha256`, `native_events_sha256`,
  `native_config_sha256` and `native_registry_sha256`. Original native capture
  receipts validate config, event and provenance hashes. Historical native
  observer hashes remain those of their capture.
- `bootstrap_requests` is explicit. Native synchronous `getChunk` calls retain
  order, duplicates and their `create` flags. Bootstrap rows have negative
  request indices, `phase: bootstrap`, `snapshot: null` and `watch: []`.
  They collect inert coverage, adding no scripted save observations. Scripted
  request indices start at zero; the unchanged Java probe uses `create=true`.
- For each scripted request, **watches run first**, in configured order. They
  observe the canonical holder through `chunk_access`, `chunk_snapshot` and
  `progress`. A converted holder crosses `prepare_chunk_save(game_time)`;
  a proto watch copies pending tags and packs saved queues without loading BEs.
  Their `snapshot_origin` is `holder_latest`.
- The requested snapshot then calls **that returned object's**
  `prepare_save(game_time)`. It serializes `saved.view.chunk` and the returned
  saved BE/tick data, with `snapshot_origin: returned_object`. A cached OLD Proto
  remains Proto even when the preceding watch saved the holder's LevelChunk.
- `raw_object_kind` retains `Proto`, `ImposterProto` or `Level`. Imposters use
  wrapped level storage and map to LevelChunk **only for serialization/class**,
  matching `HistoryProbe.snapshot`. `snapshot_boundary` is `proto_save` or
  `level_save`. `persisted_status` and `serialized_status` are the saved view's
  status. The comparator checks them separately against native's later observed
  status and its NBT `Status`, since native work may advance between them.
- Every observation has its own position. Explicit `absent: true` scores chunk
  presence; a missing snapshot is an error. Missing, duplicate, reordered or
  wrong-position watches are rejected, as are mismatched request/phase indices,
  specs, origins, input hashes and referenced native NBT positions.

The shared snapshot comparator scores requested **and** watch states, quart
biomes, WG/runtime heightmap presence/values, serialized sky/block light,
logical structures, saved BE payloads and `keepPacked`, ordered saved tick
entries, per-section postprocessing, class and statuses. **LIGHT/SPAWN variation
is never normalized.** Every measured mismatch is retained in `differences`,
including unequal-length tails; `first` and `examples` are previews only.

`comparison.json.requests` retains the requested score fields at their original
locations and adds ordered `watch` scores. `requested_scored_matches` applies
only to that returned snapshot; the row's `scored_matches` also includes watches
and FEATURES source order. Top-level `scored_matches` aggregates those rows.
`matches: false` means a measured mismatch; `matches: null` leaves full parity
unproven even when measured fields agree. A watch can report an exact match of
**absence only**. Missing optional observations and unsupported/unknown fields
are explicitly unscored. Typed NBT widths/wire order, saved-vs-live heightmap
filtering, `isLightOn`, entities and world registration are outside these scores.

`complete`, `coverage_status`, ordered stage states/missing reasons and feature
coverage remain implementation diagnostics. In particular, a LevelChunk's FULL
coverage can remain **Partial**. Saved queue contents do not establish tick
registration or execution. Native write attribution retains the feature,
source, event and RNG state when available, bounded by the request-exit event.
The native watch has no separate timestamp for exact concurrent write attribution.

To compare further captures against **the same frozen BCore build**:

```powershell
python scripts/run_generation_check.py --background native-history-reuse -- python -B scripts/native-generation-reference/replay_bcore.py target/native-history/other-capture --output target/native-history/other-bcore-run --reuse-build target/native-history/my-new-bcore-run --timeout 3600
```

Wait for the fresh job to finish successfully before submitting dependent reuse
jobs. Reuse checks the exact program/version, successful compile receipt,
binary hash, `source.zip` hash and every archive member against `source_sha256`,
the previous plan and its input hashes, and previous tool provenance. Missing,
extra, duplicate or noncanonical archive members fail validation. The copied
binary must advertise the schema 3 contract via `--version` before replay.

`source.zip` retains the build's production core, registry and worldgen
Rust/JSON/manifests/lockfile and
toolchain plus the then-current observer Rust, Python, Java and README sources.
Archived Cargo path dependencies are checked before compiling and on reuse;
historical builds keep their own dependency graph rather than acquiring current
registry inputs.
It stays byte-identical on reuse. Each invocation also writes **`tools.zip`**
with the current Python tools, README and serialized check runner, recording
`tool_sha256` and `tools_zip_sha256`. `runner_sha256`/`comparator_sha256` identify
those current invocation tools, independently of the frozen build. Binary,
plan, native identity and invocation tools are checked again after execution.

Source order and optional feature-boundary tracing remain available:
`--trace-feature "0,0,minecraft:patch_grass_forest"` records
`feature_traces.jsonl` with the original production RNG continuation and chunk
snapshots. Reused traced builds require `feature_trace_version: 1`.

## Causal forest-grass diagnostic

```powershell
python scripts/native-generation-reference/capture.py --output target/native-history/grass-native --feature-snapshots patch_grass_forest --request "0,0,biomes" --request "0,0,noise" --request "0,0,surface" --request "0,0,carvers" --request "0,0,features"
python scripts/native-generation-reference/feature_case.py target/native-history/grass-native --output target/native-history/grass-native/case.json
python scripts/native-generation-reference/replay_bcore.py target/native-history/grass-native --feature-case target/native-history/grass-native/case.json --output target/native-history/grass-bcore
```

The native feature snapshots include complete before/after data for its nine
write-square chunks. The diagnostic runs the **production placement/base-feature
implementation** with the captured states, palettes/BiomeZoom and RNG state,
varying only the heightmap callback. It supports this specific simple-block
feature and rejects unexpected effects/queries rather than filling in them.

Observed result for the primary seed/source `(0,0)`:

| Height callback | Different blocks across 9 chunks | Ordered writes | Native nextLong |
|---|---:|---|---|
| Retained native `WORLD_SURFACE_WG` | **0 / 884736** | All 5 exact | **-2473347630682788069**, exact |
| Rescan current live states | **6** | 1 instead of 5 | Same exact value |

Two of the five native writes go to neighbours. The four center differences
are the first shared-context divergence after otherwise exact requested
BIOMES/NOISE/SURFACE/CARVERS boundaries. Native `WORLD_SURFACE_WG(15,6)` returns
**126**, although the current live predicate gives **133**; at `(6,5)` the values
are **127** versus **128**. This is a retained-heightmap API issue, not a random
stream mismatch or a simple-block-kernel fix.

This retention is status-dependent: the native ProtoChunk has six live maps
after FEATURES, but conversion to LevelChunk at FULL carries only the four
final maps. `oracle.py height-history <capture>` reports this transition; the
WG maps must not simply be frozen forever across FULL/loading.

## Fixtures and verification

Portable immutable fixtures under `fixtures/` package raw successful captures,
their source/provenance, complete snapshot blobs and a per-file digest manifest.
The native worlds themselves are unnecessary for replaying these observations.

```powershell
python scripts/run_generation_check.py --background native-history-python -- python -B scripts/native-generation-reference/test_oracle.py -v
python scripts/run_generation_check.py --background native-history-rustfmt -- rustfmt --check --edition 2021 scripts/native-generation-reference/replay.rs
python scripts/native-generation-reference/oracle.py native-diff target/native-history/direct --other target/native-history/reverse
python scripts/native-generation-reference/pack_fixture.py target/native-history/my-new-run --output scripts/native-generation-reference/fixtures/a-new-name.zip
```

The tests check native execution, FULL versus gameplay ticking, complete palette
decoding against native getters, retained WG observations, exact RNG continuation,
ordered cross-chunk writes, corruption/truncation rejection and a real native
history counterexample. Four identical FEATURES sources in opposite request
orders differ in **1129** states of `(0,0)`, first `(2,-52,0)` sculk versus
deepslate. Independent same-history recapture matched all four final raw NBT
blobs exactly.

Schema 3 contract tests additionally exercise exact i64/version handling,
bootstrap call preservation, old-Proto versus canonical-Level watches, absent
and missing observations, duplicates/order/position rejection, metadata and
LIGHT/SPAWN differences, unscored fields, complete mismatch retention, and
binary/archive/manifest/plan/tool reuse validation. Synthetic contract cases
are temporary test inputs, independent of the immutable native fixtures.

Observer update receipt: `target/native-request-replay-handoff-20261015-01.md`.
Rustfmt verifies syntax/formatting; it does not compile or type-check the Rust
observer. Production replay integration against N1, N2 and B1 remains pending
the coordinator's fresh build and subsequent reuse runs.

Do not overwrite successful native captures or edit fixture values to fit
BCore. Fix and document an actual probe bug before obtaining replacement
evidence, always in a new directory.

## Scope still requiring validation

The corpus covers fresh Overworld histories, seeds 846692123413862008, 0 and 42,
origin/distant/negative/corner coordinates, adjacent/reversed/repeated requests,
native initial-spawn bootstrap and actual stages through FULL. Default runs
use one background worker. It does not establish arbitrary concurrent ticket
histories, unload/reload/persistence, gameplay postprocessing/ticking, Nether/End,
or whole-world parity. GenerationWorld's explicit Partial/missing-work reports
are retained in every BCore diagnostic result.

Detailed handoff, all case paths and first divergences:
`target/full-parity-20261002/native-history.md`.
