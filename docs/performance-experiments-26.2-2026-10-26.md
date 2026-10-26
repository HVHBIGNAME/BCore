# Seeded NOISE cache experiments — 2026-10-26

Both candidates were **rejected**. The production source tree was restored to the
verified pre-experiment snapshot: **510 byte-identical benchmark inputs**,
manifest SHA-256 `34883593c2747fcf91eb9a82bb7cfd7f35aca83dc12a009ce2939f51583f8184`.
The new creative pillar functionality is part of that baseline and remains intact.

[Versioned measurements](metrics/noise-cache-experiments-26_2-2026-10-26.json)
contain **126 matched process records / 1,260 warmup / 1,260 measured batches**,
every process median, frozen source/binary identities and both restoration proofs.
Full durations, requests and logs remain in the hash-identified local raw results.

## Hypotheses and numerical checks

The October 24 profile attributed 596 leaf samples to `ImprovedNoise::noise`, 551
to generic density evaluation and 645 to indexed interpolation. Source review
also found two borrowed-name lookups on every seeded normal-noise cache hit.
Those observations select a hypothesis; they do not quantify lookup cost.

The [October 9 lookup prototype](performance-experiments-2026-10-09.md) combined
single-hit lookup with `FastHasher` changes and was rejected. This experiment
isolated lookup/storage changes on the current **26.2** implementation after the
accepted outlining/indexed-density work. Neither candidate changed the hasher,
seed/name identity, noise arithmetic, RNG consumption or interpolation order.

1. **Hit-return:** replace `contains_key` plus `get` with an early-return borrowed
   lookup. The seeded resource stays in the original nested maps. Cold insertion
   still precedes sampling. Sampling appears at separate hit/miss call sites.
2. **Dense index:** store `name → usize` in the per-seed map and `NormalNoise` in
   an append-only vector, using one common sampling tail. This reduces map-value
   size, but total RSS and the cause of timing variation were not measured.

Each candidate passed **21 executed regression tests**: nine indexed density
bit/order/unwind tests, two cache tests, six numeric reference tests, one native
ore-material test, two historical configured-ore tests and one historical RNG
test. The numeric target fixture remains **712 cases / 51,003 exact values for
26.2**; historical 26.1 authorities retain their version.

Cache witnesses compare cold/warm sampling and bare/namespaced aliases against
uncached factories, including exceptional IEEE values and four isolated threads.
The dense variant exercises every bundled noise name, forward/reverse insertion
order, map/vector growth and five seeds including `i64::MIN`/`i64::MAX`.

The initial hit-return gate used a nonexistent `rng_reference` target and failed
explicitly. Its 18 executed tests are retained as incomplete evidence. The
corrected gate uses `random_reference` and also adds `ore_reference`; it completed
all 21 tests before the candidate benchmark started. No empty filter is accepted.

## Matched Java/BCore measurements

Every full run uses `noise-fill-wg-v2`, seed `846692123413862008`, the same eight
chunks, Y=-64..319, workers 1/2/4, three fresh processes per engine/configuration,
ten warmup and ten measured batches. Setup and timer boundaries are unchanged;
only private frozen default-release binaries are measured.

| Snapshot | Workers | BCore chunks/s | Java 26.2 chunks/s |
|---|---:|---:|---:|
| Before | 1 | 53.5995 | 54.3264 |
| Before | 2 | 94.6078 | 96.6350 |
| Before | 4 | 176.5531 | 201.0070 |
| Hit-return | 1 | 51.2473 | 52.7501 |
| Hit-return | 2 | 90.5596 | 76.4598 |
| Hit-return | 4 | 109.1530 | 215.5114 |
| Dense index | 1 | 45.7900 | 39.9553 |
| Dense index | 2 | 89.1617 | 84.1961 |
| Dense index | 4 | 161.9514 | 193.4072 |

These separate-run differences are not accepted BCore-relative speedups. Java
and BCore both vary on this unisolated host; the paired comparisons below keep
the old/new binaries alternating within each session.

## Two alternating frozen confirmations per candidate

Ratios are candidate throughput divided by before throughput, computed from the
median of three process medians. Each confirmation contains 18 fresh BCore
processes; it does not remeasure Java.

| Candidate | Workers | Run 1 ratio | Run 2 ratio | Decision |
|---|---:|---:|---:|---|
| Hit-return | 1 | 0.979400× | 1.055294× | Mixed |
| Hit-return | 2 | 1.023678× | 1.059585× | Two local gains |
| Hit-return | 4 | 0.953915× | 0.987303× | Both regress |
| Dense index | 1 | 1.085696× | 0.969648× | Mixed |
| Dense index | 2 | 0.909616× | 1.020923× | Mixed |
| Dense index | 4 | 1.033618× | 1.004519× | Small local gains |

Neither candidate has repeated improvement at all three budgets. Both source
changes and prototype-only tests were removed. The apparent one/two-worker Java
wins in individual full runs do not close [the Java goal](https://github.com/HVHBIGNAME/BCore/issues/2).

All 126 processes match the same eight block-state, ordered-mark,
`WORLD_SURFACE_WG` and `OCEAN_FLOOR_WG` fingerprints. Matching outputs establish
the measured kernel boundary, not complete generation or server/gameplay parity.
No new production speedup is claimed. Fixed warmup convergence, scheduling and
the source of the variation remain unestablished.

## Retained local evidence

- `target/noise-cache-before-20261025-01/`: fresh baseline; results SHA-256
  `8e4c8e44b19bb3683ba0cad13551fe0593200dc158089d74f1c28f826d1c43b2`.
- `target/noise-cache-single-20261025-01/`: hit-return candidate; results SHA-256
  `8c6bee74ec1381b0cadc6da1c562a32aa23ad09beae4edd10251f992cc4d2052`.
- `target/noise-cache-paired-20261025-01/` and `-02/`: its two confirmations.
- `target/noise-cache-tests-20261025-02/`: corrected 21-test gate.
- `target/noise-cache-restoration-20261026-01.json`: first byte-restoration proof.
- `target/noise-cache-dense-20261026-01/`: dense candidate; results SHA-256
  `6c0e73a43dfa4d2fc86a75aa32aa2ec8137ef9aff1c5bda48a1345edecf88b50`.
- `target/noise-cache-dense-paired-20261026-01/` and `-02/`: its two confirmations.
- `target/noise-cache-dense-tests-20261026-01/`: 21-test all-name/growth gate.
- `target/noise-cache-dense-restoration-20261026-01.json`: final restoration proof.

The restored `simplex.rs` SHA-256 is
`e3e181f8047b2f7c9d12f0d64e7452828917c38ee9ffba6bb65e1c54709207b1`.
The frozen source archives retain both prototypes for review. Historical baseline
charts and measurements are preserved with their original identities.

## Reproduce

Use new output names and the ROOT heavy-work queue. The production code now
contains neither rejected prototype; recover a prototype only from its verified
source archive before reproducing that comparison.

```powershell
python -B scripts/run_generation_check.py --background noise-26_2-NEW -- python -B scripts/benchmarks/run.py --minecraft 26.2 --output target/noise-26_2-NEW --workers 1 2 4 --processes 3 --warmup 10 --batches 10
python -B scripts/run_generation_check.py --background noise-paired-NEW -- python -B scripts/benchmarks/compare_frozen.py --before target/verified-before/results.json --after target/verified-candidate/results.json --output target/noise-paired-NEW
```
