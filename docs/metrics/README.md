# Published measurements

The README and project site use versioned JSON evidence from this directory.
Charts are rendered locally with `python scripts/render_readme.py`; `--check`
verifies that the committed SVGs still match the data.

- `checkpoint-2026-10-08.json`: **662 passing workspace tests**, **88 requests / 14
  native histories**, 40 matching scored light snapshots and the fossil/pyramid
  before/after comparisons. All histories use one frozen replay executable;
  remaining 36 NBT fields and 70 WG-presence differences are retained.
- `checkpoint-2026-10-05.json`: previous 628-test, 69-request / 11-history checkpoint,
  preserved with its original known differences and artifact hashes.
- `noise-fill-2026-10-05.json`: independently timed original Vanilla and BCore
  material-fill kernels, with full block/effect fingerprints and run conditions.
- `../parity-live-generation.json`: the older three-region sample, preserved with
  its own executable and capture provenance. It is a different benchmark from the
  matching-history checkpoint, so the two percentages are not a trend line.

Raw JVM logs, captures and build snapshots are retained locally under `target/`.
The published exports retain their SHA-256 hashes. Reproduce new measurements in
new directories; do not rewrite previous measurements with new expected values.

`scripts/export_checkpoint.py` accepts only completed successful workspace runs.
It verifies source archives against their manifests, binary and replay output
hashes, native provenance and identical captures for before/after comparisons.
For the October 8 checkpoint, the local audit also checked every frozen source
against the final workspace. The replay executable SHA-256 is
`04ea394a71af64ad1764341448a10fb545c791725d6e79093262ff95394b9a96`;
the frozen source archive SHA-256 is
`e87a51c4fdd6cbf0c714713d69bbcc6e5dc4fee368a724c3e00b1b9610ef2612`.

Performance reproduction and interpretation: [benchmark guide](../performance.md).
