# Proposed private admission observer

Status: **proposal only**. The existing shared Java observer and its contract
are unchanged. Current baseline inputs are:

| File | SHA-256 |
|---|---|
| `native-generation-reference/HistoryAgent.java` | `f36e81ff13bba0b7dd5069d602a724d0ce2839b8bcc90e8c74dc961d182efecc` |
| `native-generation-reference/HistoryProbe.java` | `1dba9a48dfb09831da580b4ccaa3c34c9395a5162f16b793839e306a63953013` |
| `native-generation-reference/NativeAccess.java` | `93f32d9a715ea0b82abfe262c5362efc0ad08c25cfa3c028d3c5bc792c5fd55f` |

## Missing measurements

The existing observer records entry to `getChunk` and
`getChunkFutureMainThread`, but their exits are ignored by `HistoryProbe.exit`.
It records generation dispatcher handoffs, not the `BlockableEventLoop` main
queue. FULL stage entry occurs before its supplier is queued, and stage exit
observes the returned future's completion barrier. Ticket level appears on
holder events, without the ticket insertion or old/new accessibility decision.

The bytecode explains these operations, but runtime identities would make
the pump/cache explanation directly testable without relying on an inferred
association between an API wait and a later stage completion.

## Bounded hooks

A private companion transformer can observe the following methods while
leaving their native bodies, arguments, queues and returned futures intact:

| Method | Observation |
|---|---|
| `ServerChunkCache.getChunk` entry/return | Per-thread call ID, exact requested status, returned object identity/class/status, count of nested future-path calls |
| `ServerChunkCache.getChunkFutureMainThread` return | Returned future ID, `isDone`, exceptional/cancelled state; no join or extra continuation |
| `ServerChunkCache.storeInCache`, `clearCache` | Ordered four-slot object/status/position references before and after mutation |
| `ServerChunkCache.runDistanceManagerUpdates` | Successful update result and enclosing API call ID; aggregate false/no-work polls |
| `ChunkHolder.updateFutures` | Watched holders' old ticket level, new ticket level, allowed status and full-future IDs before/after |
| `ChunkMap.prepareAccessibleChunk` | Holder position/identity and returned future ID, linked to the enclosing holder update |
| `BlockableEventLoop.schedule(Runnable)` | Only the `ServerChunkCache$MainThreadExecutor` receiver: runnable ID, submitting thread and short native caller stack |
| `ServerChunkCache$MainThreadExecutor.doRunTask` | Same runnable ID at actual body entry/return, enclosing API wait ID |
| `BlockableEventLoop.managedBlock` | Only the same executor: already-known requested future's readiness at entry/exit and aggregate poll counts |
| `ChunkStatusTasks.lambda$full$0` | Input proto/holder identity and returned LevelChunk identity, linked to the currently executing main runnable |

Observe future readiness with `CompletableFuture.isDone` only. Do not invoke
the supplied BooleanSupplier an additional time, replace returned futures,
add a completion barrier, wrap/reorder native runnables, wait for extra work,
serialize extra chunks or perform heightmap/BE lookups.

Use `==`-based identity bookkeeping, as the baseline observer does. Restrict
holder detail to the requested/watch positions and their immediate neighbours.
Record aggregate no-work polling rather than every busy-wait iteration.

## Provenance and comparison

- Build in a new `target/implicit-full-native-20261015-admission-*` directory.
  Archive the baseline Java bytes and every companion/runner source before
  compilation; verify the baseline hashes above or explicitly version the
  change. Include class/agent/ASM/JAR/config hashes and exact commands.
- Keep supplemental events in a versioned sidecar. Each sidecar row should
  carry its own sequence and the baseline sequence sampled under the
  `HistoryProbe.class` emission monitor. Do not relabel baseline stage IDs.
- Reuse the seven requests in `scenario.json` unchanged and terminate at its
  final return, retaining any in-flight declarations. Execute through
  `scripts/run_generation_check.py --background`; no nested queue acquisition.
- Match main submission/execution by runnable ID and API waits by returned
  future ID. Keep admission, supplier entry, completion and snapshot boundaries
  distinct. A submission hook at method entry is not yet proof of insertion;
  record its return too to bound concurrent dequeue observations.
- Compare to N1/N2 using source order, requested/watch objects and effect
  fields. Added observation can alter timing; a moved phase label alone is
  not a behavior regression. Record actual snapshot differences rather than
  normalizing them away.

If any shared `HistoryAgent.java`/`HistoryProbe.java` edit is desired instead
of a private companion, return that exact patch to the coordinator first.
