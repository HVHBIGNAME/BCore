# Native implicit FULL: admission, waiting and cached objects

## Finding

In the pinned 26.1 server, a LIGHT cache miss with `create=true` adds an
UNKNOWN ticket at **level 33**. Crossing into full accessibility schedules a
FULL generation task and INITIALIZE_LIGHT for its eight neighbours. The API
still waits for the **LIGHT future**, rather than the FULL future. The FULL
body is a separate main-executor job. Later waits can execute that job; a
cache hit or an already-completed requested future does not drain the queue.

The four-entry API cache retains actual object references. FULL replaces the
holder's lower-status futures, but an existing cache entry can still return
the old ProtoChunk. Holder-latest, returned object, allowed status, generation
target and persisted status are distinct state.

This is a native investigation and an implementation handoff. It does not
establish BCore parity or change production/replay code.

## Evidence, 2026-10-15

Paths below are relative to the repository root. `seq` is an evidence coordinate
in that capture's `events.jsonl`; it is not an elapsed time or global causal ID.

| ID | Artifact | Result |
|---|---|---|
| N1 | `target/implicit-full-native-20261015-01` | New seven-request capture, verified |
| N2 | `target/implicit-full-native-20261015-02` | Independent new server/world, verified |
| B1 | `target/full-boundaries-native-20261014-01` | Existing verified explicit-FULL/cache contrast |
| J1 | `target/implicit-full-native-20261015-bytecode-01` | Pinned class bytecode and per-class digests |
| J2 | `target/implicit-full-native-20261015-bytecode-02` | Additional ticket classes and verbose lambda bindings |
| A2 | `target/implicit-full-native-20261015-analysis-02` | Summaries, linked partial orders and repeat comparison |
| H1 | `target/implicit-full-native-20261015-handoff-01/verification.json` | Bounded fixture assertions passed |

N1 and N2 each contain **436,374 events, 1,619 verified blobs, 12 FEATURES
sources, 784 stage entries and 783 completed stages**. Both end with one
declared in-flight FULL stage for `(1,0)`. Both start with zero holders and
end with game time zero and zero gameplay ticks. This is the intended return
boundary, not a drain-to-idle capture.

All **21 request/watch pairs** have identical uncompressed raw NBT and extra
blob digests across N1/N2. Ordered FEATURES sources also agree. Native source,
JAR, ASM, Java and library identities agree; process-dependent object IDs and
dispatcher interleavings are not used for cross-process identity.

The existing checkpoint preserves the seven requests in `scenario.json`.
It puts all 36 BE-payload differences and two WG-presence differences for
this history on request 6. The 70 WG-presence differences are the whole
checkpoint total. Its old raw capture/configuration is unavailable. These
new runs use the current primary-seed/before-spawn settings; they reconstruct
the requested scenario, not the lost results.

### Durable queue receipts

All listed jobs finished with exit code zero. Their `.json`, `.log` and
`.runner.log` files are under `target/generation-checks/`.

| Operation | Job ID |
|---|---|
| N1 capture | `implicit-full-native-20261015-01-1792070802245541900` |
| N2 capture | `implicit-full-native-20261015-02-1792070802245541900` |
| N1 full oracle verification | `implicit-full-verify-20261015-01-1792071217376089800` |
| N2 full oracle verification | `implicit-full-verify-20261015-02-1792071218365996700` |
| J1 | `implicit-full-javap-20261015-01-1792071016497844200` |
| J2 | `implicit-full-javap-20261015-02-1792071541128541300` |
| A2 | `implicit-full-analysis-20261015-02-1792072009333719300` |
| H1 | `implicit-full-handoff-verify-20261015-01-1792072544454763200` |

N1 events SHA-256:
`12e7a1c6ead7f4b4c375c5c548b2d89a4f4cef508098ae57c963c050f82b314d`.
N2 events SHA-256:
`2fe3cd327baa896a408da4a7816cf42a689b187029db4d3ae3ee3b8f62190e9d`.

The common server JAR SHA-256 is
`a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`.
The original observer is unchanged; A2 additionally checks its frozen source
files against each capture's provenance. Agent JAR digests are recorded per
build and are not assumed equal across processes.

### Observed request boundaries

Request numbers are zero-based and describe these fixtures only.

| Request | Requested API | Returned snapshot in both runs | Origin holder watch |
|---|---|---|---|
| 0 | `(0,0,FEATURES)` | ProtoChunk / FEATURES | FEATURES |
| 1 | `(0,0,INITIALIZE_LIGHT)` | ProtoChunk / INITIALIZE_LIGHT | INITIALIZE_LIGHT |
| 2 | `(0,0,LIGHT)` | ProtoChunk / SPAWN | LIGHT |
| 3 | `(1,0,FEATURES)` | ProtoChunk / INITIALIZE_LIGHT | SPAWN |
| 4 | `(1,0,INITIALIZE_LIGHT)` | ProtoChunk / INITIALIZE_LIGHT | SPAWN |
| 5 | `(1,0,LIGHT)` | ProtoChunk / SPAWN | LevelChunk / FULL |
| 6 | `(0,0,LIGHT)` | LevelChunk / FULL after probe unwrap | LevelChunk / FULL |

The watch at request 2 sees LIGHT while the later requested snapshot sees
SPAWN on the same proto object. `HistoryProbe.run` takes watches first, then
the requested snapshot; neither the row nor all the snapshots are an atomic
cross-thread boundary. The return table describes serialized observations,
not necessarily the status at the instant `getChunk` returned.

### Linked observations and variability

- **N1:** origin FULL is requested at seq `52424` with ticket level 33 during
  LIGHT/request 2. FULL stage `336546` starts on `Worker-Main-1`; its matching
  exit is seq `339133` on `Server thread` during request 5.
- **N2:** origin FULL stage `336543` starts during request **3**, after SPAWN
  stage `336528` crossed the request-2/request-3 observation boundary. Its
  matching exit is seq `339169`, again on `Server thread` during request 5.
  Scheduling originated at the LIGHT admission; the later request label is
  not a different causal trigger.
- Both runs leave neighbour FULL stage `436365` in flight. In N1 its
  `task_wait` at `436372` reports `done=false`; request 6 then returns at
  `436373`, followed by the terminal prefix at `436374`.
- **B1:** origin FULL stage `334571` starts during LIGHT/request 1 and
  completes at `334580` during explicit FULL/request 2. At `334589`, the
  subsequent LIGHT request returns original proto object **8**, persisted
  SPAWN, while the watch sees LevelChunk **1191**, FULL. There is no
  `getChunkFutureMainThread` call for that request (`334588` is `getChunk`).

A2's partial-order files retain same-thread edges in the selected scheduler
spine, stage-ID entry/completion edges and explicit
`(dispatcher,runnable)` submission/handoff edges. N1 has 167 matched runnable
submissions and no unmatched handoffs or remaining dispatcher submissions.
`dispatcher_execute` names an observer hook at `scheduleForExecution` entry;
it is not proof that the runnable body has finished. The pending FULL main
job is on a different executor, not a missing worldgen-dispatcher handoff.

## Native algorithm

The following rules are established by pinned bytecode, with the above runtime
observations corroborating their effects. J1/J2 contain complete method bodies
and provenance. Method offsets below refer to bytecode offsets, not file offsets.

### 1. Ticket level comes from the FULL dependency pyramid

`ChunkLevel.byStatus(ChunkStatus)`, offsets 0-10:

```text
ticketLevel(status) = 33 + GENERATION_PYRAMID[FULL].getAccumulatedRadiusOf(status)
```

The original `feature_dependencies_26_1.json` FULL step gives:

| Requested status | Added ticket level |
|---|---:|
| EMPTY, STRUCTURE_STARTS | 44 |
| STRUCTURE_REFERENCES, BIOMES | 36 |
| NOISE, SURFACE, CARVERS | 35 |
| FEATURES, INITIALIZE_LIGHT | 34 |
| LIGHT, SPAWN, FULL | 33 |

`ChunkLevel.generationStatus(level)` allows FULL at level <=33; at 34 it
allows INITIALIZE_LIGHT, at 35 CARVERS, at 36 BIOMES, and at 37-44
STRUCTURE_STARTS. Levels above 44 are disallowed. This is an allowed ceiling,
not a demand to execute every allowed status.

`LoadingChunkTracker` obtains the source loading-ticket minimum from
`TicketStorage`. `ChunkTracker` relaxes all eight neighbours with edge cost
one. For these fresh retained tickets, the settled level is the minimum of
`ticket.level + max(abs(dx), abs(dz))`, capped by the unloaded boundary.
The UNKNOWN type has timeout 1 and flags 18 (loading + can-expire-if-unloaded);
these histories never advance the native purge/gameplay clock.

### 2. Admission and requested-status demand are separate

`ServerChunkCache.getChunkFutureMainThread`, offsets 18-154:

```text
level = ticketLevel(status)
holder = visibleHolder(pos)
if create:
    add UNKNOWN(pos, level)
    if holder is absent or holder.ticketLevel > level:
        runDistanceManagerUpdates()
        holder = visibleHolder(pos)
        require holder exists and holder.ticketLevel <= level
if holder is absent or holder.ticketLevel > level:
    return UNLOADED_CHUNK_FUTURE
return holder.scheduleChunkGenerationTask(status, chunkMap)
```

`DistanceManager.runAllUpdates` first runs loading-distance changes. It then
does **two passes** over changed holders: `updateHighestAllowedStatus` for
all, followed by `updateFutures` for all. Preserve that ordering.

`ChunkHolder.updateFutures`, offsets 0-84, detects an old inaccessible -> new
FULL-accessible transition and calls `ChunkMap.prepareAccessibleChunk`.
That requests a radius-one range: centre FULL, eight neighbours
INITIALIZE_LIGHT. J2's bootstrap method 33 binds the selector explicitly to
`ChunkLevel.getStatusAroundFullChunk`; the range loops z outside x. The
original API then requests its own status future, which can be LIGHT.

`GenerationChunkHolder.scheduleChunkGenerationTask`, offsets 0-63, returns
an already-completed future immediately. Otherwise it creates/reschedules
only if there is no task or the requested target exceeds the current task's
target. A LIGHT demand therefore reuses an already-scheduled FULL task.
Existing status claims/futures prevent duplicate stage execution.

### 3. Completion pumping is conditional

`ServerChunkCache.getChunk` has a four-slot `(packedPos, exactStatus)` cache.
A matching non-null entry returns directly. On a miss it gets the requested
future, runs `managedBlock(future::isDone)`, joins, and pushes the returned
object into slot zero. Slots shift only on stores; hits do not promote them.

`BlockableEventLoop.managedBlock`, offsets 10-30, checks the guard **before**
polling and again after each poll/wait. An already-complete future means no
poll. It does not wait for all native tasks or for maximum admitted status.

`ServerChunkCache.MainThreadExecutor.pollTask`:

```text
if runDistanceManagerUpdates():
    return true
lightEngine.tryScheduleUpdate()
return poll one FIFO main-executor runnable
```

`runDistanceManagerUpdates` runs distance updates, promotes the visible map,
then flushes `pendingGenerationTasks` to the native worldgen dispatcher.
If distance processing or map promotion changed anything, it clears all
four cache entries and returns true. Flushing tasks happens even when it
returns false. Native worldgen workers and the light dispatcher progress
independently; completed waits enqueue task continuations.

`ChunkStatusTasks.full`, offsets 25-41, calls `CompletableFuture.supplyAsync`
on `WorldGenContext.mainThreadExecutor`. This executor's
`scheduleExecutables()` always returns true, so the FULL body is queued,
including when submitted from the main thread. Its `lambda$full$0` performs
the actual LevelChunk conversion and holder-future replacement.

Thus a later request that must wait can execute an older FULL job. A cache
hit, or a miss returning an already-completed lower-status future, can leave
that same job pending. These are state/future rules, independent of request
number, coordinates or LIGHT as a special conversion trigger.

### 4. Object and save boundaries survive conversion

`GenerationChunkHolder.replaceProtoChunk` replaces each lower-status future
slot with a completed future containing a read-only ImposterProtoChunk. It
does not rewrite the existing API cache or previously returned proto objects.
`HistoryProbe.snapshot` unwraps imposters before serializing them.

Across N1's origin FULL before/after snapshots, A2 measures **36 logical
BE-payload field changes**, two retired WG maps, unchanged states/biomes,
and exactly retained saved ticks/postprocessing. Nine DUMMY tags become
eight sculk sensors and one shrieker; an existing chest is retained.
The origin retains **616 saved block ticks and 1,847 postprocessing marks**.
`keepPacked` is a separate envelope, not counted in the 36 payload changes.

These stage snapshots include serialization. The separate native
`scripts/full-lifecycle-reference` fixture establishes that pending tags
survive conversion and are materialized by LevelChunk saving. This capture
alone does not isolate the pre-save BE maps.

## Smallest implementation slice and regression cases

Keep the existing generation step bodies, dependency pyramids and conversion
implementation. Add the API/scheduler state needed to represent:

1. Loading ticket minima, allowed ceilings and old/new accessibility.
2. Status futures plus an outstanding generation target for each holder.
3. Pending generation tasks, existing worker/light dispatch and a FIFO main
   executor, with explicit future readiness and the native conditional pump.
4. A four-slot status-keyed returned-object cache with native invalidation.
5. Retained proto handles versus converted holder storage and read-only
   imposters; save the object actually selected by each API boundary.

Concrete regressions tied to the evidence:

| Case | Evidence / expected invariant |
|---|---|
| Pyramid-derived levels | J1 `ChunkLevel`, original FULL dependency data: FEATURES=34, LIGHT=SPAWN=FULL=33; no ordinal-distance shortcut |
| Allowed is not demanded | N1/N2 request 0: ticket 34 still returns FEATURES; INITIALIZE_LIGHT is allowed but not automatically executed just for permission |
| LIGHT admits FULL without awaiting it | N1/N2 request 2: level-33 FULL task exists; requested snapshot is still ProtoChunk |
| Ready futures do not drain | N1/N2 requests 3/4: already-generated neighbour FEATURES/INITIALIZE_LIGHT return while origin remains proto |
| A later wait services old work | N1 stage `336546` / N2 `336543` completes on the main thread during neighbour LIGHT/request 5 |
| Terminal ready return preserves pending work | Request 6 returns origin FULL; neighbour FULL stage `436365` remains in flight |
| Cache identity can outlive holder replacement | B1 request 3 returns proto 8/SPAWN while watch reads level 1191/FULL; no future API call |
| Miss after invalidation selects replaced future | N1/N2 request 6 takes the future path and snapshot unwraps the converted origin |
| Save materialization preserves effects | N1/N2 origin transition: 36 BE-field changes, WG 6->4, same states/biomes/616 block ticks/1,847 marks |

Fixture request numbers identify assertions, not production branching rules.
Scheduler tests should drive ticket, ready-future and queued-job states and
assert these invariants. Native interleavings may place SPAWN/FULL stage entry
on either side of a request label, as N1/N2 already demonstrate.

## Remaining evidence gaps and next actions

- **Established:** pinned ticket mapping, promotion call path, conditional
  pump, cache lookup/store/clear rules and deferred FULL body; native effects
  for this seven-request scenario and the existing explicit-FULL contrast.
- **Not directly instrumented:** main-executor enqueue/run identity pairs,
  requested-future readiness at `managedBlock` entry, exact cache clear/store
  calls and the old/new accessibility fields at promotion. Static rules and
  native consequences support the explanation, but these event families are
  not present in the current observer.
- **Not established:** all possible interleavings, expiry/unload/reload,
  off-thread requests, `create=false` runtime cases or arbitrary ticket
  populations. New scheduler implementation needs its own regression results.

`private-probe-proposal.md` specifies a narrowly scoped companion observer
for the missing events. It is a proposal; neither current HistoryAgent nor
HistoryProbe was changed. Main can use N1/N2 immediately for its schema-2
replay work, while keeping returned-object and holder-watch observations
separate. Implementation and fresh replay verification remain next actions.

## Tools

`javap.py` records class-byte and disassembly digests without initializing
Minecraft classes. `analyze.py` freezes the current decoder into its new
output directory, validates selected blob/input/source digests, emits compact
request/effect summaries, and links only explicit partial-order evidence.
The full oracle verification remains a separate queued operation.
`verify_handoff.py` checks the reconstructed request list, independent snapshot
agreement, linked evidence edges, the cache contrast and the stated lifecycle
effects against these fixtures; it does not test a BCore implementation.

Example analysis, with a **new** output suffix:

```powershell
python scripts/run_generation_check.py --background implicit-full-analysis-new -- python -B scripts/implicit-full-reference/analyze.py --output target/implicit-full-native-analysis-new --capture target/implicit-full-native-20261015-01 --capture target/implicit-full-native-20261015-02 --compare-first-two
```

Every JVM, capture and verification runs through the same background queue.
The read-only delegated observer audit failed with repeated ECONNRESET and
was stopped; the findings above were obtained by this investigation.
