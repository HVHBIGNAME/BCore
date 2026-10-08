# Published measurements

The README and project site use versioned JSON evidence from this directory.
Charts are rendered locally with `python scripts/render_readme.py`; `--check`
verifies that the committed SVGs still match the data.

- `noise-fill-wg-v2-2026-10-08.json`: current NOISE material-fill measurement
  under the revised `noise-fill-wg-v2` contract, which also fingerprints both
  worldgen heightmaps. Includes the verified optimization series and its
  provenance.
- `checkpoint-2026-10-08-optimized.json`: **665 passing workspace tests**,
  **88 requests / 14 native histories**, 40 matching scored light snapshots.
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

Raw JVM logs, captures and build snapshots are retained locally under `target/`.
The published exports retain their SHA-256 hashes. Reproduce new measurements in
new directories; do not rewrite previous measurements with new expected values.

`scripts/export_checkpoint.py` accepts only completed successful workspace runs.
It verifies source archives against their manifests, binary and replay output
hashes, native provenance and identical captures for before/after comparisons.
For the optimized checkpoint, the replay executable SHA-256 is
`c4a93cd8995af7d9fd6272790ce2f937b319b3c5297e8b324d9fccc2fcf9dfe8`;
the frozen source archive SHA-256 is
`380285a15ebd818e36a297a69e9823df0979433287a2e468f8ca95e6d56bacdc`.

Performance reproduction and interpretation: [benchmark guide](../performance.md).
