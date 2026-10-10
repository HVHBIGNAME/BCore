# Published measurements

The README and project site use versioned JSON evidence from this directory.
Charts are rendered locally with `python scripts/render_readme.py`; `--check`
verifies that the committed SVGs still match the data.

- `checkpoint-2026-10-10.json`: **91 requests / 15 histories** on the published
  phase-3 executable, adding three desert-well requests. 8,945,664 state and
  139,776 biome observations match; 40 scored light snapshots match. The known
  **36 BE fields / 70 WG-presence differences** and incomplete coverage remain.
  Reuses the October 8 **665-test** workspace run on unchanged Rust sources;
  the date identifies the expanded replay export, not a new full test run.
- `noise-fill-wg-v2-2026-10-08.json`: latest measurement of the accepted NOISE implementation
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
