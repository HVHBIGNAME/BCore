# Axis-only creative block gameplay — 2026-10-26

The next Phase 2 slice adds **56 native pillar items / 168 axis states** to the
existing tick-owned creative break/place implementation. Together with the
earlier 197 stateless bindings, **253 full-cube items / 365 states** are supported.
Progress is tracked in [#5](https://github.com/HVHBIGNAME/BCore/issues/5) and the
[full requirements](roadmap.md).

## Implemented behavior

- The selected unpatched creative hotbar stack chooses the placed state.
- UP/DOWN choose native Y states, NORTH/SOUTH native Z, WEST/EAST native X.
  Explicit captured IDs are used; state IDs are not calculated by arithmetic.
- Every supported axis can be clicked as a placement support or instantly broken
  by creative `START_DESTROY_BLOCK`. ABORT/STOP do not destroy the block.
- Existing ready/mode/hand/hit/reach/world/delivered-chunk checks stay on the
  action-context snapshot. Occupied targets and standing-player overlap reject.
- Mutation remains on the tick owner, through the retained canonical Level,
  shared light updates, payload invalidation and current-subscriber publication.
- Coalesced acknowledgements follow the existing dirty BCC save boundary.
- Pending block-entity tags remain inert during dirty saving. Restart/readback
  preserves the saved axis states without rewriting the save.

The catalogue contains exact `BlockItem` → exact `RotatedPillarBlock` bindings
with only the `axis` property, empty fluid and no block entity. All three states
have a native full collision cube. It includes logs/wood, stems/hyphae, stripped
variants, deepslate, basalt and other direct pillar instances. Specialized block
subclasses are not inferred from similar names or properties.

## Authenticated native oracle

`scripts/migration-26.2/PillarPlacementReference.java` invokes the original
`RotatedPillarBlock.getStateForPlacement` for every catalogue item and all six
faces. Each selected state is encoded and decoded with the native
`ClientboundBlockUpdatePacket.STREAM_CODEC`; all payload bytes are consumed.

Two independent JVM runs match byte-for-byte: **336 cases / 56 items / 168 states**.
Target identity is **26.2 / 776 / 4903**, JAR SHA-256:

```text
183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8
```

The capture pins all **39 libraries**, the Java executable and five source helpers:
`TreeReference.java`, `NativeWorldgenRegistries.java`, `ProtocolSchema.java`,
`BlockActionReference.java`, `PillarPlacementReference.java`. The existing four
helpers and the previous 25-case fixture remain byte-identical.

- Fixture: `crates/bcore-protocol/data/pillar_placement_26_2.json` — **191,343 bytes**,
  SHA-256 `163e7acf1275fe1a7431f7f1f080152d13a2f2aa7e5d32ac9995b0880ed1f698`.
- Provenance: `crates/bcore-protocol/data/pillar_placement_provenance_26_2.json`.
- Raw repeats: `target/native-pillar-placement-20261025-01/run-1.json` and `run-2.json`.
- Native method inspections: `target/native-pillar-placement-classes-20261025-01/`.

The isolated native context is allocated with its real hit-result field populated;
its constructor and world/player callbacks are not executed. This oracle proves
the native face-to-state rule and codecs, not the entire `BlockItem.useOn` pipeline.

Examples from the native registry: oak log X/Y/Z = **136/137/138**;
deepslate X/Y/Z = **30416/30417/30418**. Rust verifies the corresponding names,
properties, default Y state and stack limits against the shared registry.

## Verification

The fresh workspace gate passed **927 tests / 0 failed / 4 ignored**, **72 suites**,
with **526 unchanged source inputs**. Format, workspace release build and standalone
replay binaries also passed. Source-manifest SHA-256:
`ddab648760008f464ca9ff5f1e95f7cfc4852585558cdb4f7ce31f1977a4c5fe`;
raw validation SHA-256:
`5f28f374885f00a800f3a9ac1f6b6cff8c2b2a0f6a76696bb0cfdb7a0567819d`.

The source-bound focused run completed **43 tests**, including three new native
fixture/binding witnesses and three new actual TCP scenarios:

1. All six oak-log faces, another viewer, retained holders, cached payload
   invalidation, a chunk-border placement, dirty saving and restart/terrain readback.
2. A negative-coordinate deepslate chain placed on Y/X/Z supports; each axis breaks.
   ABORT/STOP, mode, hand, hit, BE/unloaded and collision rejection preserve
   saves. Unsupported stairs/chain/chest stacks retain and correct the old hotbar.
3. Basalt placement across the chunk border excludes a teleported old subscriber;
   a malformed actual creative stack closes the connection without disk writes.

The focused gate also reruns seven original wire tests, five canonical mutation
tests, six earlier block TCP tests, six join and thirteen chat regressions.
Fixture tests compare every one of the **336 native payloads** with Rust, enforce
all five helper hashes and reject non-cube/BE/waterlogged families.

The initial TCP target had an ambiguous integer type, then a test attempted to
read an intentionally invalidated payload cache without rebuilding it. Both test
issues were corrected; the final source-bound run is
`target/pillar-focus-20261025-03/validation.json`. Earlier failed outputs remain
separate from the passing evidence.

[Workspace, source manifest and focused/native identities](metrics/creative-pillar-gameplay-validation-2026-10-26.json)
record the final full check. The publication image receives a fresh independent
Rust/portable/site gate; metric counters are verified against the raw logs.

## Reproduce

With the prepared native 26.2 environment, select a new capture directory:

```powershell
python -B scripts/run_generation_check.py --background native-pillar-NEW -- python -B scripts/migration-26.2/pillar_placement.py --output target/native-pillar-NEW
python -B scripts/run_generation_check.py --background pillar-wire-NEW -- cargo test -p bcore-protocol --release --locked -j 1 --lib blocks::pillar_tests -- --test-threads=2
python -B scripts/run_generation_check.py --background pillar-tcp-NEW -- cargo test -p bcore-protocol --release --locked -j 1 --test pillar_actions -- --test-threads=2
```

`--publish` is create-new only; the committed native fixture must not be overwritten
with a recapture. Tests use owned temporary worlds, not user worlds/operators.

## Remaining boundaries

This is creative, main-hand, unpatched-hotbar, air-target, standing-player gameplay.
Menus/click transactions, persisted playerdata, survival/drops, replaceable
vegetation, stairs/slabs/waterlogging, neighbour callbacks, poses/physics and
ticket-driven FULL remain open in #4. Actual TCP tests are not a GUI-client proof.

The simultaneous [NOISE cache experiments](performance-experiments-26.2-2026-10-26.md)
were both rejected after 126 matching processes and repeated frozen comparisons.
Their production source changes were restored; this gameplay checkpoint does not
claim a generation/server speedup. Local `aislop` remains unavailable; no full
scan score is claimed.
