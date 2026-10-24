# Loot and retained-chest validation — 2026-10-18

Finalized on 2026-10-19. Dates in artifact names identify the frozen checkpoint;
individual queue receipts retain their actual execution times.

BCore now contains the native-backed loot interpreter and a single-chest
lazy-inventory boundary on the retained level chunk. The supported inputs are
Minecraft **26.1 / protocol 775**, null-player context, positive-zero luck and
explicit nonzero seeds, or an explicitly caller-owned random source.

This milestone covers generation of items/components/slots, one-time unpacking,
native saved-NBT projections and BCC snapshot persistence. In-game player menus,
automatic inventory saving and loaded-holder hydration remain separate work.

## Implemented path

- `bcore_worldgen::loot::LootEngine` decodes/compiles the built-in catalogs once.
  Table evaluation preserves pool/entry/function order, native registry/tag order,
  float32 arithmetic, required and skipped RNG draws, stack splitting and slot
  shuffling. Registered unsupported tables fail explicitly; only absent keys use
  native EMPTY behavior.
- `block_entity::ChestInventory` represents 27 backing slots, a deferred loot key
  and its retained seed. Persistent items use validated item/component codecs and
  exact numeric/list NBT types. Unsupported data is not converted to empty items.
- `GenerationWorld::unpack_chest(pos, (x, y, z))` accesses an existing single chest
  on an already-converted level chunk under the generation mutex. The coordinates
  are chunk-local. `unpack_chest_with_rng` consumes exactly the supplied source;
  it does not select or initialize a server random sequence.
- `World::unpack_generated_chest(x, y, z)` exposes that boundary using absolute
  block coordinates. The caller uses `prepare_generated_chunk_save` and the store
  for explicit persistence. This method does not load or generate missing chunks.

The key is cleared before filling. Preflight failures preserve the entity and
source. A late evaluation error commits the consumed key before reporting the
error, preventing an implicit reroll. Repeated no-key access performs no map
replacement or RNG draw. Eight concurrent readers are tested against one fill.

The seed remains in runtime `template_load_data` after unpacking. Native saved
NBT omits both the key and seed and contains ascending Byte-slot `Items`. A fresh
load of that materialized NBT defaults its seed to zero and does not generate
loot again. Loading deferred NBT ignores its `Items`, as the native chest does.
Accepted slot-order/key/list representations are normalized when materialized,
without requiring an inventory access first.

An existing generated dungeon-chest member acquires the typed inventory
representation while remaining visible through the old proto's live membership.
A pending tag loaded only on the level does not become a member of the retained
old proto. Old returned objects, holder futures and detached snapshots keep their
existing identities and save boundaries. Removal/recreation at the same position
still requires the separate per-entity ownership work.

## Native authority

Pinned original server JAR SHA-256:

```text
a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52
```

The reusable runtime consumes six portable catalog exports, with 1,326 built-in
tables and 1,506 item definitions. These inventories are not a claim that every
table or component vocabulary is implemented.

| Reference | Verified scope | SHA-256 |
| --- | --- | --- |
| `loot-fill-26.1.json` | 137 cases, 57 registered tables, 1,292 occupied slots | `2158fd855f1bbc46d6b399620c9ced35822263c982151686f58e175c1ccf9db1` |
| `chest-lifecycle-26.1.json` | Five cases, six native snapshots each, two independent matching native runs | `4b0bc865d02713dc62b50b2d5a9a6c28c742ee54af64e88461965e98ed1de8b0` |

Both fixtures are embedded from
`crates/bcore-worldgen/tests/data/loot_26_1/`; runtime code does not read test cases
or `target/` captures. The original lifecycle evidence archive is
`target/chest-lifecycle-native-20261018-fixtures/evidence.zip`, SHA-256
`8a877972baaafe4fb372d0fd0551a101df3e7828201720c4a4ff3ba594482638`.

The fresh native cases cover dungeon loot, a negative-seed enchanted book, preset
component-bearing items, table-over-Items load precedence, and a missing table.
Each native run observed four original fills and ten fresh actual chest loads.
The probe used original `BlockEntity.loadStatic`, `saveWithFullMetadata` and
`NbtIo` methods on real server-attached chests. Reopen used `unpackLootTable(null)`
and all 27 getters. No player menu or gameplay tick was exercised.

The original fill fixture additionally checks generated stack order, effective
components, maximum stack sizes, every setter, RNG phase checkpoints and exact
continuation bits. Its 134 actual 27-slot chest records contain 127 deferred-empty
loads and seven preinstalled backing inventories. The other three cases test
generic zero/one-slot fills, not single-chest lifecycle. The chest tests separately
round-trip 1,291 occupied native chest slots.

## Validation

**782 passed, zero failed, four existing ignored tests, 66 test/doc-test targets.**
This is a fresh full-workspace release run on the integrated source snapshot.
The [loot/chest metrics](metrics/loot-chest-2026-10-18.json) contain completed
receipts, log hashes, source identities, target totals and per-test timings.

The full run includes:

- 19 loot tests plus one actual API doctest in MAIN.
- 18 `chest_inventory` tests, including all 30 fresh native snapshots.
- Nine owner tests; the earlier focused generation/FULL/request run also passed
  all 107 selected regressions.
- Three BCC persistence tests: five raw pending inputs, ten live before/after
  representations and five fresh materialized loads; byte-stable BCC re-encoding
  and 135 guarded no-reroll slot reads.

Owner fixtures exercise the five native histories from both pending and live
starting maps. Initial-save assertions use separate worlds so they cannot mask
the direct pending-to-unpack path. Complete canonical entity equality guards
no-key reads; separate tests cover old-proto membership, unsupported preflight,
single-chest/coordinate boundaries, concurrent access and late errors.

The pre-validation source snapshot contains **417 inputs**, including dirty and
new files. Archive SHA-256:
`2d57e69e4b5ca97e41953270e192b42bd8b534fa7d0670e18c3b39a758e9088b`.
Manifest SHA-256:
`c0620409c512382d1bb846046d5fa328b2aa01737a7dad74ef579bcf5838f486`.
The accepted density implementation remains
`5d66c4e33e40e7a9eb3d34ddb61ce51bf84444dc0be762c5cf190423a33d5cf1`.

All 417 current source hashes still match the pre-validation snapshot. Workspace
formatting, workspace release build, the diagnostics feature check and the built
server's `--help` invocation passed. The four ignored tests are the existing JVM
bridge test, two manual benchmarks and `parity_fixture_matches_bcore`.

The full test job took 1,410.99 seconds: Cargo reported 1,060 seconds of build time;
completed harness times sum to 296.85 seconds. These are validation costs.

## Fresh native request replay

The same four histories were replayed with a **new schema-3 executable**, preserving
the actual returned-object and ordered holder-watch contract. The last three
runs reused that verified frozen executable, rather than rebuilding between histories.
The [request metrics](metrics/native-request-2026-10-18.json) retain all scored
differences and unscored fields.

| History | Requested scored matches | Watch scored matches |
| --- | ---: | ---: |
| N1 — `implicit-full-native-20261015-01` | 7/7 | 13/14 |
| N2 — `implicit-full-native-20261015-02` | 7/7 | 13/14 |
| B1 — `full-boundaries-native-20261014-01` | 6/6 | 12/12 |
| Native repeat — `implicit-full-native-20261015-admission-seven-02` | 7/7 | 14/14 |
| Total | **27/27** | **52/54** |

Requested snapshots compare 2,654,208 block-state and 41,472 biome observations;
watches separately compare 5,308,416 and 82,944. Both groups have zero differences
in these values. Scored block-entity payloads/packing, maps, ticks, structures,
marks and source order also match. All 23 requested and 46 watch light snapshots
with applicable scoring match.

The existing four lifecycle status-field differences remain in two origin watches
at request index 2 of N1/N2: native LIGHT versus BCore SPAWN for persisted and
serialized status. The independent native-repeat history matches. No differences
were normalized away. All 27 requests still report Partial implementation coverage.
Logical chunk-NBT scoring does not establish numeric-tag widths; the separate
loot/chest comparisons explicitly check those widths for their covered inventories.

## Current NOISE measurement

The unchanged `noise-fill-wg-v2` workload ran on the same 417-input source snapshot:
eight chunks, 10 warmup batches and 10 measured batches per process, three fresh
processes for each engine at each worker budget. All **18 process records** matched
the native block, ordered-mark and both WG-heightmap fingerprints.

| Workers | Java chunks/s | BCore chunks/s | Java variation | BCore variation |
| --- | ---: | ---: | ---: | ---: |
| 1 | 45.13 | 20.18 | 26.27% | 9.54% |
| 2 | 73.69 | 27.34 | 15.13% | 15.03% |
| 4 | 172.91 | 58.67 | 19.76% | 20.93% |

Throughput is 8 divided by the median of the three process medians. Variation is
`100 * (max - min) / median` of those process medians. All warmup and measured
samples, process medians, fingerprints and build/runtime identities are retained
in [the performance metrics](metrics/noise-fill-wg-v2-chest-2026-10-18.json).

**BCore is slower than Java in this measurement.** No new NOISE optimization was
accepted. Timing differences from older runs are not an alternating frozen
before/after result. The workload excludes later generation stages, inventory
unpacking, packets, disk I/O and gameplay; it is not a whole-server benchmark.

## Reproducible artifacts and receipts

| Artifact | SHA-256 |
| --- | --- |
| Server `target/build/release/bcore.exe` | `690b209281fc268423bea20a0af049169b9aee80a625256724e7f075f12f7268` |
| Frozen `native-history-replay.exe` | `77eb26d4c5bc88cf887f462c4e46fd58dbcd01214771d58f1352b97617f3aec2` |
| Frozen benchmark `bin/noise_bench.exe` | `d705c3012e11b48db30c67003d4d940e93839038758bf20272caf2b3203abeb4` |
| Raw benchmark `results.json` | `39fa9f594b21076349228de1959ef7a22cabd52fe524599c94d40dc3e820105e` |

Every operation used ROOT `scripts/run_generation_check.py`; queue receipt and
log paths are `target/generation-checks/<job>.{json,log}`. Final jobs:

| Check | Job ID | Exit |
| --- | --- | ---: |
| Source snapshot | `chest-checkpoint-20261018-freeze-1792346581286689000` | 0 |
| Full release tests | `chest-checkpoint-20261018-workspace-1792346678728572400` | 0 |
| Workspace formatting | `chest-checkpoint-20261018-format-1792346679782393000` | 0 |
| Workspace release build | `chest-checkpoint-20261018-build-1792346679783895800` | 0 |
| Diagnostics check | `chest-checkpoint-20261018-diagnostics-1792346679782393000` | 0 |
| Server help | `chest-checkpoint-20261018-server-help-1792412387477056400` | 0 |
| NOISE measurement | `chest-checkpoint-20261018-noise-1792347265447319600` | 0 |
| N1 replay/build | `chest-checkpoint-20261018-replay-n1-1792347266420859500` | 0 |
| N2 replay | `chest-checkpoint-20261018-replay-n2-1792412387477056400` | 0 |
| B1 replay | `chest-checkpoint-20261018-replay-b1-1792412387477056400` | 0 |
| Repeat replay | `chest-checkpoint-20261018-replay-repeat-1792412387082291000` | 0 |
| Loot/checkpoint export | `chest-checkpoint-20261019-export-loot-1792412434846389500` | 0 |
| Request export | `chest-checkpoint-20261019-export-requests-1792412596968101300` | 0 |
| Performance export | `chest-checkpoint-20261019-export-noise-1792412782432859200` | 0 |

The earlier owner-test compile failure (incorrect test NBT accessor) and formatting
failures remain in their original receipts. They were corrected before this frozen
validation. Tests and captures were not weakened or replaced. Queue exit zero for
a replay means comparison completed; the 52/54 result above is still a mismatch.

## Remaining boundaries

- Player/luck/advancement/access checks and network container menus/clicks/closing.
- Server named-sequence/level-RNG ownership and persistence for optional seed zero.
  A supplied source is supported, but is not automatic vanilla source selection.
- Map-dependent loot (`exploration_map`/`set_name`), other component codecs,
  double chests and non-chest container lifecycles.
- Automatic inventory disk writes and loading saved chunks into live generation
  holders. BCC tests prove representation compatibility, not native process restart.
- Remaining Partial FULL/generation coverage and known request-watch status
  differences. Bounded loot equality does not establish whole-world parity.
- Java-relative performance: test-suite durations are validation timings. This
  integration does not introduce a new accepted NOISE optimization.

Native algorithm notes: [loot reference](../scripts/loot-reference/ALGORITHMS.md).
Previous request milestone:
[2026-10-16 report](native-request-validation-2026-10-16.md).
