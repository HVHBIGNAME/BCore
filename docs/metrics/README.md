# Published measurements

The README and project site use versioned JSON evidence from this directory.
Charts are rendered locally with `python scripts/render_readme.py`; `--check`
verifies that the committed SVGs still match the data.

- `checkpoint-2026-10-05.json`: completed workspace test totals and per-request
  native-history comparisons, including known NBT/lifecycle differences.
- `noise-fill-2026-10-05.json`: independently timed original Vanilla and BCore
  material-fill kernels, with full block/effect fingerprints and run conditions.
- `../parity-live-generation.json`: the older three-region sample, preserved with
  its own executable and capture provenance. It is a different benchmark from the
  matching-history checkpoint, so the two percentages are not a trend line.

Raw JVM logs, captures and build snapshots are retained locally under `target/`.
The published exports retain their SHA-256 hashes. Reproduce new measurements in
new directories; do not rewrite previous measurements with new expected values.

Performance reproduction and interpretation: [benchmark guide](../performance.md).
