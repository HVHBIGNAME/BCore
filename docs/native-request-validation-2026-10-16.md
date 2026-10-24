# Native request/runtime checkpoint — 2026-10-16

The integrated working tree passes **732 Rust tests across 61 test/doc-test
targets**, with zero failures and four ignored tests. The release workspace,
worldgen diagnostics and server `--help` check pass.

The new request runtime preserves returned-object identity across FULL
conversion. Four schema-3 replays share one frozen executable: all **27 requested
snapshots** match their scored native fields; **52 of 54 holder watches** match.
The two remaining watch differences are recorded LIGHT/SPAWN status observations.
FULL implementation coverage remains **Partial**.

Detailed counts, commands, raw-artifact hashes and unscored fields are in
[`metrics/native-request-2026-10-16.json`](metrics/native-request-2026-10-16.json).

## Production changes

- Native ticket levels and distance propagation admit work independently of
  the requested status. Accessibility, status and FULL-supplier futures have
  separate identities.
- The four-slot request cache retains the selected objects. Replacing holder
  futures during FULL does not rewrite an old ProtoChunk cache entry or a held
  future. Readiness controls whether the owner thread services its FIFO queue.
- `World::request_native_chunk` exposes the owner-thread request route through
  the same production `GenerationWorld`. `ReturnedChunk::view` observes that
  object; `prepare_save` saves it, including an OLD Proto after holder conversion.
- Shutdown, poisoning and worker/main unwind close outstanding waiters.
- Proto saves preserve pending NBT verbatim and prefer existing live entities.
  Only level saving adds the `keepPacked` envelope. The initial replay exposed
  the incorrect shared envelope; pinned bytecode and a new runtime regression
  establish the corrected boundary. See the
  [native method evidence](../scripts/implicit-full-reference/proto-save.md).

Existing network/payload consumers still use the generation-only route. Native
request admission currently requires its owner thread; callers must not mix
generation-only FULL calls into a native request history.

## Measured histories

These are fresh, before-spawn overworld histories at seed
`846692123413862008`, with game time zero. Counts include repeated observations
of the same chunks and air; they are not unique-chunk or feature-coverage counts.

| Native reference | Requested snapshots matching scored fields | Holder watches matching scored fields |
|---|---:|---:|
| N1, `implicit-full-native-20261015-01` | 7/7 | 13/14 |
| N2, independent `implicit-full-native-20261015-02` | 7/7 | 13/14 |
| B1, explicit FULL / OLD Proto cache contrast | 6/6 | 12/12 |
| Independent companion-observed seven-request repeat | 7/7 | 14/14 |

Across requested snapshots, **2,654,208 block-state and 41,472 biome
observations** match. Holder watches separately contribute **5,308,416
block-state and 82,944 biome observations**, also with zero differences.

All scored WG/runtime-map presence and values, structures, BE payloads,
`keepPacked`, saved tick order, postprocessing and FEATURES source order match.
Serialized light matches in all **23 requested / 46 watch** snapshots where
it is scored. The other light observations remain explicitly unscored.

### Remaining status observations

In N1 and N2, request 2's origin watch records native `minecraft:light`; BCore
records `minecraft:spawn`. Both the saved NBT status and the later observed
status differ: **four field differences across two watches**. No blocks,
biomes, maps or BE/tick payloads differ at those boundaries.

The already-captured independent native companion repeat records SPAWN at the
same watch and matches all scored BCore observations. It used the native queues
and futures without an added completion barrier. Instrumentation timing and
native scheduling effects are not isolated by these captures. N1/N2 remain
mismatches in the published result; their status values were not normalized or
replaced with the matching repeat.

`scored_matches` covers only measured fields. `matches: null` does not assert
complete NBT or FULL parity. Numeric NBT widths, compound wire order,
`isLightOn`, world registration and other listed fields remain outside these
snapshot comparisons. All 27 requests retain incomplete implementation coverage.
The older 91-request checkpoint and its 36/70 difference totals keep their
historical scope; these four histories do not replace that wider corpus.

## Verification receipts

All jobs used `scripts/run_generation_check.py`. Status and full output are
under `target/generation-checks/<job>.json` and `<job>.log`.

| Check | Job | Result |
|---|---|---|
| Final workspace release tests | `native-request-workspace-release-20261016-02-1792121761491146300` | 732 passed, 0 failed, 4 ignored |
| Workspace formatting | `native-request-workspace-format-20261016-02-1792121761478555000` | Passed |
| Observer formatting | `native-request-observer-format-20261016-02-1792121761511682100` | Passed |
| Workspace release build | `native-request-workspace-build-20261016-01-1792122175516352700` | Passed |
| Worldgen diagnostics, all targets | `native-request-diagnostics-20261016-01-1792122175516352700` | Passed |
| Built server `--help` | `native-request-server-help-20261016-01-1792123356850241500` | Passed |
| Python observer contracts | `native-request-replay-python-20261015-01-1792117563773494200` | 41 passed |
| Checkpoint exporter contracts | `native-request-exporter-tests-20261016-01-1792119874161613600` | 13 passed |
| Fresh corrected N1 replay | `native-request-replay-n1-20261016-02-1792121761599903400` | Executed and compared |
| Corrected N2 reuse | `native-request-replay-n2-20261016-02-1792123356824217300` | Executed and compared |
| Corrected B1 reuse | `native-request-replay-b1-20261016-02-1792123356823216700` | Executed and compared |
| Companion-repeat reuse | `native-request-replay-admission-repeat-20261016-01-1792123356823216700` | Executed and compared |
| Verified evidence export | `native-request-final-export-20261016-01-1792123565058373900` | Passed |

The Python observer tests preceded the final Rust-only proto-save correction;
the final native replays compile and exercise that correction. Replay exit zero
means execution/comparison completed, not that every measured field matched.

The final workspace run took **1,382.56 seconds**: Cargo reported **1,087 seconds**
of build time; completed harness wall times totalled **253.16 seconds**. These
are validation timings, not throughput benchmarks. Pre-existing dead-code and
deprecated-helper compiler warnings remain.

Artifact identities:

- Server: `target/build/release/bcore.exe`, SHA-256
  `eb1e12e431f86ad870c87ee317656419fd79120d90bf66a1b81c821c379fd38e`.
- Frozen replay: `target/native-request-replay-n1-20261016-02/native-history-replay.exe`,
  SHA-256 `44dcca6cdd11b23281bd04198585b14874f0f737b8b78b5d45766a74105af907`.
- Frozen replay source archive: SHA-256
  `51a5af27ae404a029f3896352b475b965cc74c0db93b3bcfa20ff91ab3013d67`.
- Compiler: `rustc 1.100.0-nightly (8925ea358 2026-08-20)`.

The initial 731-test/N1 export, including the packing error, remains in
[`metrics/native-request-initial-2026-10-16.json`](metrics/native-request-initial-2026-10-16.json).
Schema-3 plans, executable contracts, native-input hashes, source/tool archives
and reuse provenance are validated by the runner and checkpoint exporter.

## Performance decision and next integration work

The four-entry density cell-cache experiment was rejected after alternating
frozen comparisons regressed throughput at every worker budget. Its code and
prototype-only tests were removed; the accepted shared-corner implementation
matches the FULL-base snapshot exactly. The
[separate experiment report](performance-experiments-2026-10-15.md) preserves
the measurements. This request/runtime checkpoint does not claim a new Java
speedup or whole-server benchmark.

Remaining integration includes native asynchronous load/noise/light
continuations, off-thread request dispatch and networking owner-thread routing;
post-load entities, live BE/tick registration and level full-status/unsaved
callbacks; expiry/unload/reload and saved-holder hydration; and per-entity
identity across removal/recreation. Wider structure/feature and dimension
coverage also remains incomplete. Future checks should exercise those actual
transitions rather than force the request-label timing of one capture.
