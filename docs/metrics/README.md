# Published measurements

The README and project site use versioned JSON evidence from this directory.
Charts are rendered locally with `python scripts/render_readme.py`; `--check`
verifies that the committed SVGs still match the data.

- `runtime-latency-validation-2026-10-27.json`: **939 / 0 / 4**, 72 suites,
  868 unchanged source inputs, 4592 protected world/operator files; 12 new regressions.
  Three 32-action TCP bursts per revision: **1881.04 → 243.79 ms median**. Two alternating
  production comparisons retain 20 TPS windows, correction p95 **1718/1781 → 141/187 ms**,
  483 matching terrain payload pairs, higher 93 → 125 ms median and reduced chunk delivery.
  Sustained 488 place/break actions with 171 chunks preserve ordered updates, durability
  acks and final saved AIR. Includes executable/archive/driver/log hashes and the
  interrupted earlier workspace attempt. [Report](../runtime-latency-2026-10-27.md).
- `creative-pillar-gameplay-validation-2026-10-26.json`: **927 / 0 / 4**, 72 suites
  and 526 unchanged source inputs; **43 focused tests / 3 new actual TCP scenarios**.
  Native pillar authority: **336 repeat-matching cases / 56 items / 168 states**;
  together with the prior stateless slice, 253 full-cube items / 365 states are supported.
  Retains complete workspace suites, focused test names/log hashes and all five
  native helper identities. [Report](../creative-pillar-gameplay-2026-10-26.md).
- `noise-cache-experiments-26_2-2026-10-26.json`: two rejected seeded-cache candidates,
  **126 matching processes / 1260 warmup / 1260 measured batches**, **21 bit/order/native
  tests per candidate**, two frozen confirmations each. Hit-return regresses at four
  workers; dense index has mixed one/two-worker outcomes. All 510 baseline inputs
  restored byte-exactly; no new production gain. Preserves every process median,
  raw/frozen identities and both restoration proofs. [Report](../performance-experiments-26.2-2026-10-26.md).
- `noise-axis-delta-rejected-26_2-2026-10-25.json`: rejected first-axis delta
  candidate, **72 matching process records / 720 warmup / 720 measured batches**,
  10 executed bit/order and 9 native numeric/ore/RNG tests. Two alternating frozen
  comparisons give **0.999032× / 0.992884×** at four workers; source files restored
  byte-exactly. Preserves all process medians, raw-result and frozen artifact hashes.
  See the [experiment report](../performance-experiments-26.2-2026-10-25.md).
- `creative-block-gameplay-validation-2026-10-25.json`: **921 passed / 0 failed /
  4 ignored**, 71 workspace suites and 517 unchanged source inputs. Includes all
  suite results/log hashes, **37 focused tests**, **6 actual TCP gameplay scenarios**,
  **25 repeat-matching native codec cases** and **197 stateless block-item bindings**.
  Tick-owned creative edits, viewer updates, dirty saves and restart are exercised;
  full physics, menus, survival and light/ticket lifecycle remain open. See the
  [gameplay report](../creative-block-gameplay-2026-10-25.md).
- `saved-chunk-admission-validation-2026-10-25.json`: the earlier **903 / 0 / 4**,
  70-suite canonical saved-data checkpoint, including 13 admission regressions.
  Imported generation history stays Partial with zero attempts.
- `storage-cleanup-2026-10-24.json`: **18.76 GiB** of expendable files removed,
  including **33 verified expanded-source copies** and debug incremental cache.
  Original source archives, binaries, captures and logs remain; **2,958 protected
  world/operator files** have identical before/after bytes. The observed volume
  free-space delta is reported separately from logical deleted bytes.
- `noise-fill-wg-v2-baseline-26_2-2026-10-24.json`: current authenticated **26.2 /
  776 / 4903** baseline, BCore **47.79 / 83.98 / 156.81** chunks/s versus Java
  **49.77 / 74.03 / 215.35** at 1/2/4 workers. All **18 process records**, **180
  warmup / 180 measured batches**, blocks, ordered marks and both WG maps match.
  Preserves the raw-result hash, all durations and **489-input** frozen provenance;
  supplies `performance-26_2.svg`. Host variation remains visible; no cross-version
  optimization gain is claimed.
- `migration-workspace-validation-2026-10-24.json`: complete **886 passed / 0
  failed / 4 ignored**, 69-suite workspace release run, including the published
  bootstrap regression. Keeps every suite and exact log hash. Supplies the test
  count in `migration-26_2.svg`; its protocol counts come from the final versioned
  native validation below.
- `protocol-26_2-validation-2026-10-24.json`: native bootstrap/schema proof on the
  final rebuilt frozen server, **1179 / 1179 frames / 127 terrain packets** across
  three sessions. Binds both original byte blobs and authenticated official captures,
  session UUID ownership, clean shutdowns and the saved edit observed after restart.
  Server SHA-256 `4e6cb4e410d241d8e40c36362a5faa65a21f1418380a6f61717f28bd5bf38400`.
  Preserves the earlier `crates/bcore-protocol/data/protocol_26_2.json` unchanged.
- `noise-cpu-profile-26_2-2026-10-24.json`: fresh owned-child diagnostic, **4,002
  leaf samples**, exit 0, matching native baseline fingerprints. Retains the complete
  symbol/source summaries, artifact hashes, debug-line override, request and sampler
  limitations/errors. These sampled timings are excluded from throughput. See the
  [current profile report](../performance-profile-26.2-2026-10-24.md).
- `noise-fill-wg-v2-final-2026-10-21.json`: historical **26.1** outline/indexed/borrowed
  implementation, **39.60 / 71.13 / 140.00** chunks/s versus Java **44.16 / 88.43 /
  198.41** at 1/2/4 workers. All 18 full-run process records match; this file supplies
  the historical `performance.svg` chart. Java remains faster in this bounded run.
- `noise-fill-wg-v2-series-2026-10-21.json`: all seven retained/rejected experiments,
  378 process records with warmup/measured batches and per-process medians. Two
  **direct cumulative frozen comparisons** against the October 18 chest checkpoint
  show **2.02–2.45× gain** across 1/2/4 workers; all 18 individual pairs improve.
  Includes the small mixed-pair borrowed-point gain and rejected cell-miss,
  constant-operand and Perlin-rounding variants. See the
  [final experiment report](../performance-experiments-2026-10-21.md).
- `noise-final-validation-2026-10-21.json`: fresh **791 passing tests**, zero
  failures, four ignored tests and 66 targets; format/release/build/diagnostics/
  server-help checks and four histories on one newly built schema-3 replay.
  **27/27 requested, 52/54 watch** scored matches; all comparison facets match the
  preceding checkpoint, including the four known LIGHT/SPAWN fields. All 419
  source inputs match the retained frozen implementation; coverage stays Partial.
- `noise-fill-wg-v2-outline-2026-10-20.json`: accepted selective density outlining,
  **24–39% repeatable BCore gain** across two alternating confirmations at 1/2/4
  workers; all 18 before/after pairs improved. Same-run BCore **24.22 / 41.25 /
  85.23** chunks/s versus Java **42.28 / 92.28 / 175.28**. All 54 process records
  matched. See the [profile and experiment report](../performance-experiments-2026-10-20.md).
- `noise-outline-validation-2026-10-20.json`: fresh **782 passing tests**, zero
  failures, 66 targets; release/build/diagnostics checks and four new schema-3
  histories. **27/27 requested, 52/54 watch** scored matches, with the same known
  status-field differences and Partial coverage.
- `loot-chest-2026-10-18.json`: fresh **782 passing Rust tests**, 66 targets,
  zero failures and four existing ignored tests. Includes the 137-case loot
  interpreter, native chest load/unpack/reopen comparisons, retained owner
  integration and BCC persistence checks. All 417 source inputs match the frozen
  snapshot. See the [loot/chest report](../loot-chest-validation-2026-10-18.md).
- `native-request-2026-10-18.json`: four histories replayed with a new shared
  schema-3 build after loot integration; **27/27 requested** and **52/54 watch**
  scored matches. The four known LIGHT/SPAWN status-field differences and Partial
  generation coverage remain. Final replays/export completed on October 19.
- `noise-fill-wg-v2-chest-2026-10-18.json`: pre-series matching-output measurement,
  preserving all 18 process records and medians. BCore **20.18 / 27.34 / 58.67**
  chunks/s versus Java **45.13 / 73.69 / 172.91** at 1/2/4 workers. Java is faster;
  no new NOISE optimization or whole-server performance claim follows.
- `native-request-2026-10-16.json`: **732 passing Rust tests**, four schema-3
  histories using one frozen replay build, **27/27 requested** and **52/54 watch**
  snapshots matching their scored fields. Four LIGHT/SPAWN status-field
  differences across two watches remain; FULL stays Partial. See the
  [request/runtime validation report](../native-request-validation-2026-10-16.md).
- `native-request-initial-2026-10-16.json`: initial 731-test/N1 evidence before
  correcting the proto `keepPacked` envelope. Its recorded differences remain
  historical; it is not the final request/runtime checkpoint.
- `noise-fill-wg-v2-cell-hit-paired-2026-10-15.json`: rejected four-entry
  interpolated-cell front cache. Preserves every process median and frozen
  artifact identities from the alternating comparison; throughput ratios were
  **0.776× / 0.932× / 0.858×** at 1/2/4 workers, with matching outputs. The
  prototype was removed; accepted shared-corner reuse remains. See the
  [experiment notes](../performance-experiments-2026-10-15.md).
- `workspace-validation-2026-10-14.json`: fresh **677-test** validation of the
  uncommitted aquifer/density work, successful release/diagnostic builds and
  eleven reverified archived native FULL relationships. See the
  [validation notes](../validation-2026-10-14.md); this is not a new native replay
  checkpoint or throughput measurement.
- `checkpoint-2026-10-10.json`: **91 requests / 15 histories** on the published
  phase-3 executable, adding three desert-well requests. 8,945,664 state and
  139,776 biome observations match; 40 scored light snapshots match. The known
  **36 BE fields / 70 WG-presence differences** and incomplete coverage remain.
  Reuses the October 8 **665-test** workspace run on unchanged Rust sources;
  the date identifies the expanded replay export, not a new full test run.
- `noise-fill-wg-v2-2026-10-08.json`: October 8 measurement of the phase-3 NOISE implementation
  under the revised `noise-fill-wg-v2` contract, which also fingerprints both
  worldgen heightmaps. Contains **final phase 3 only**, with its samples and
  provenance.
- `noise-fill-wg-v2-series-2026-10-08.json`: compact summary of the first and
  repeated v2 baselines and all three optimization passes. Preserves per-process
  medians for both engines at 1 / 2 / 4 workers, raw-result hashes, recorded
  source/binary identities and references to their shared contract/environment.
- `noise-fill-wg-v2-flat-grid-2026-10-09.json` and
  `noise-fill-wg-v2-flat-grid-paired-2026-10-09.json`: rejected dense-cache
  prototype, full native/BCore benchmark and alternating old/new BCore comparison.
- `noise-fill-wg-v2-noise-lookup-2026-10-09.json` and
  `noise-fill-wg-v2-noise-lookup-paired-2026-10-09.json`: rejected lookup prototype,
  with mixed throughput across worker counts. Each file preserves all 18 process
  samples and original result hashes; neither prototype is in production.
- `checkpoint-2026-10-08-optimized.json`: **665 passing workspace tests**,
  **88 requests / 14 native histories**, 40 matching scored light snapshots.
  Blocks and biomes match exactly in those snapshots; **36 block-entity field
  differences and 70 WG-presence differences remain**, with no complete requests.
  Its before/after rows are carried from the earlier published checkpoint because
  those frozen replays were pruned after publication.
- `checkpoint-2026-10-08.json`: earlier 662-test checkpoint of the same histories.
- `checkpoint-2026-10-05.json`: previous 628-test, 69-request / 11-history checkpoint,
  preserved with its original known differences and artifact hashes.
- `noise-fill-2026-10-05.json`: independently timed original Vanilla and BCore
  material-fill kernels under the earlier `noise-fill-kernel` contract, with full
  block/effect fingerprints and run conditions. The two performance series have
  different timed boundaries and are not directly comparable.
- `../parity-live-generation.json`: the older three-region sample, preserved with
  its own executable and capture provenance. It is a different benchmark from the
  matching-history checkpoint, so the two percentages are not a trend line.

Raw JVM logs, captures and build snapshots were stored locally under `target/`;
historical artifacts may be pruned. Published SHA-256 hashes identify those
artifacts but are not a downloadable replay bundle. The compact series retains
process medians, not all raw batches or build inputs. Reproduce new measurements
in new directories; do not rewrite previous measurements with new expected values.

`scripts/export_checkpoint.py` accepts only completed successful workspace runs.
It verifies source archives against their manifests, binary and replay output
hashes, native provenance and identical captures for before/after comparisons.
For the optimized checkpoint, the replay executable SHA-256 is
`c4a93cd8995af7d9fd6272790ce2f937b319b3c5297e8b324d9fccc2fcf9dfe8`;
the frozen source archive SHA-256 is
`380285a15ebd818e36a297a69e9823df0979433287a2e468f8ca95e6d56bacdc`.

Performance reproduction and interpretation: [benchmark guide](../performance.md)
and [rejected experiments](../performance-experiments-2026-10-09.md).

Hashes referencing published JSON (`carried_from_sha256` and the series'
`shared_reference.sha256`) use its Git UTF-8/LF bytes, as required by
`.gitattributes`. Native capture, raw-result and binary hashes remain byte-exact
hashes of their original local artifacts; Windows CRLF is not stripped from them.
