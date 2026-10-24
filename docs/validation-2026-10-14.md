# Fresh integrated validation — 2026-10-14

The working tree passed **677 Rust tests across 60 test/doc-test targets**, with
zero failures and four intentionally ignored tests. The release server builds,
its `--help` command runs, and all worldgen targets compile with `diagnostics`.

This validates the current uncommitted aquifer-bound and shared-density changes
on top of `05329feaec0bdf0553e04d4154c8c5e29bc7f429`. The workspace test ran on
October 13; the build, diagnostic check and archived FULL verification completed
on October 14. Commands, job IDs, hashes and limitations are recorded in
[`metrics/workspace-validation-2026-10-14.json`](metrics/workspace-validation-2026-10-14.json).

## Results

| Check | Result |
|---|---|
| Workspace release tests | 677 passed, 0 failed, 4 ignored |
| Workspace release build | Passed; `target/build/release/bcore.exe` |
| Worldgen `diagnostics`, all targets | Passed |
| Rust formatting | Passed |
| Windows queue behavior | 8 tests passed |
| Cargo timing parser | 3 tests passed |
| Archived native FULL relationships | 11/11 passed |

The full suite includes all five aquifer-bound tests, seven shared-density tests,
five terrain-context tests, both density-material tests and the numerical native
reference tests. It also includes all twelve chat-flow tests with the existing
join-notice synchronization changes.

The four ignored tests require a local JVM/plugin, an external parity fixture,
or explicitly requested benchmarking. Existing dead-code/deprecated-helper
compiler warnings remain. No new throughput improvement has been measured.

The cold workspace run took 1,083.99 seconds: Cargo reported 659 seconds of build
time, and completed test-target wall times totalled 387.50 seconds. The remaining
37.49 seconds are unattributed. Parallel per-test timings are not summed as
elapsed runtime; `scripts/test_timings.py` retains both kinds of measurement.

## Validation runner

`scripts/run_generation_check.py` now publishes status JSON atomically and retries
brief Windows file-sharing conflicts. Concurrent-reader tests reproduced both
the original empty-JSON race and replacement failures while a reader held the
file open. Foreground/background jobs, argument forwarding, serialization and
lock release after failures pass the behavioral checks.

All heavy checks used the shared queue in `target/generation-checks/`, one Cargo
build job and two Rayon workers. JSON status and child output are stored as
`<job>.json` and `<job>.log`. The FULL probe's reproduction commands now use this
persistent runner rather than the deleted worktree-local script.

## Archived FULL evidence

The original `full-lifecycle-26_1-v1.zip` was extracted byte-for-byte and checked
with its archived verifier. Both captured observations still agree, all eleven
native relationships pass, and a fresh verification archive was produced at
`target/full-lifecycle-reverified-20261014-new.zip`.

These are observations of the original Minecraft 26.1 FULL task. Re-verifying
them does not execute a new JVM capture or establish BCore FULL parity. The prior
matched-history checkpoint and its remaining differences retain their original
scope. Native recapture needs the lost Java/server/ASM dependencies restored.

## Next production milestone

Implement retained conversion for explicitly admitted FULL work, then a distinct
level-save operation. The current materialization epilogue in
`GenerationState::generate` is still keyed only to the requested FULL status.

1. Add real proto/converted holder state through `run_stage`; keep it separate
   from coverage and reuse it for repeated FULL requests.
2. Give lower-status reads a read-only converted view backed by the same live
   data. Retain old proto WG metadata and redirect converted WG queries to runtime
   maps. Preserve the native highest-block versus first-free-height distinction.
3. Transfer prepared ticks to retained `LevelTickQueue::from_saved` containers,
   without unpacking or executing them. Preserve the original duplicate-bearing
   `TickRequest` history alongside that prepared representation.
4. Materialize pending block entities at explicit level serialization on retained
   storage, not at conversion or in a disposable snapshot. Keep ordinary snapshot
   inspection and BCC v5 encoding inert.

Regression tests must exercise the real converter, even when terrain is supplied
by `test_stage`, and cover before-save pending tags, live entity preservation,
read-only lower-status access, tick preparation, duplicate marks and idempotence.
The native probe and existing tick/BE reference tests provide the component
evidence. Implicit ticket-level-33 FULL during LIGHT, world registration,
post-load entities, ticking readiness and saved-holder hydration remain separate
integration work; this milestone must continue to report incomplete coverage.
