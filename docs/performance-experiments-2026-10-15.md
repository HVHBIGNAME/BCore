# NOISE cell-cache experiment — 2026-10-15

## Rejected: four-entry interpolated-cell front cache

The candidate added a four-entry, exact-key cache ahead of the existing
interpolated-cell table. Eight focused regressions and the full native/BCore
benchmark preserved the measured block, ordered-mark and WG-heightmap outputs.

The acceptance comparison alternated frozen BCore binaries within one session:
three fresh processes per variant and worker budget, eight chunks per batch,
ten warmup batches and ten measured batches. The baseline already included
explicit FULL conversion and the accepted shared-corner optimization.

| Workers | Baseline chunks/s | Candidate chunks/s | Throughput ratio |
|---|---:|---:|---:|
| 1 | 20.83 | 16.17 | 0.776× |
| 2 | 29.89 | 27.87 | 0.932× |
| 4 | 57.29 | 49.13 | 0.858× |

All 18 processes retained matching fingerprints. Throughput nevertheless fell
at every worker budget. On October 16, the prototype and its eight dedicated
tests were removed; `density.rs` was checked against the frozen FULL-base source
with `git diff --no-index` and matched exactly. Shared-corner reuse remains in
production. The candidate's frozen archive retains the rejected implementation.

[Compact measurements and provenance](metrics/noise-fill-wg-v2-cell-hit-paired-2026-10-15.json)
retain every process median, both executable/source identities and the raw-result
hash. Local artifacts:

- `target/noise-fill-wg-v2-full-base-20261015-01/results.json`
- `target/noise-fill-wg-v2-cell-hit-20261015-01/results.json`
- `target/noise-fill-wg-v2-cell-hit-paired-20261015-01/results.json`
- Queue receipt: `noise-cell-hit-paired-20261015-01-1792077576987401500`, exit 0.

The host is not CPU-isolated. All process medians, including slow samples, are
retained. This BCore-only comparison supports rejecting this implementation; it
does not measure Java throughput or establish the precise source of overhead.
Earlier operation counts are not CPU-time percentages. The attempted WPR profile
could not start because Windows rejected profiling policy (`0xc5585011`).
