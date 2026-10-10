# NOISE experiments — 2026-10-09

All measurements use the existing `noise-fill-wg-v2` workload: eight chunks,
1/2/4 workers, three fresh processes per configuration, ten warmup and ten timed
batches. No competing project benchmark/test job runs inside the heavy-work queue.
The host is not CPU-isolated, so both engines can vary between sessions.

## Rejected: lazy 5×5 FlatCache

The prototype kept all 25 quart values instead of only the previous point. It
preserved quantization, out-of-grid delegation, nested evaluations and float bits;
269 library tests passed, including three prototype-specific tests. All benchmark
block, ordered-mark and WG-map fingerprints matched the native reference.

It nevertheless regressed in a same-session alternating comparison against the
frozen, previously published phase-3 executable:

| Workers | Phase 3 chunks/s | Prototype chunks/s | Throughput ratio |
|---|---:|---:|---:|
| 1 | 8.49 | 7.57 | 0.892× |
| 2 | 16.70 | 15.17 | 0.908× |
| 4 | 31.15 | 29.31 | 0.941× |

The implementation and its prototype-only tests were removed from the working
tree. The frozen benchmark source archive retains them for investigation. The
result rejects this implementation; it does not prove that every dense-cache
design is slower or identify the precise source of its overhead.

The unchanged phase-3 executable measured 31.15 chunks/s here versus 44.14 in its
October 8 measurement. This is why old/new binaries are alternated within the
same session rather than treating historical timing differences as code effects.
No Java throughput is measured by this supplementary BCore-only comparison.

[Full native/BCore measurement](metrics/noise-fill-wg-v2-flat-grid-2026-10-09.json)
and [alternating frozen comparison](metrics/noise-fill-wg-v2-flat-grid-paired-2026-10-09.json).
Local source/binary evidence:

- `target/noise-fill-wg-v2-flat-grid-20261009-01/results.json`
- `target/noise-fill-wg-v2-flat-grid-paired-20261009-01/results.json`
- `target/parallel-20261008/main-flat-grid-lib-01-1791547162964958000.log`

## Rejected: cheaper seeded-noise lookups

This prototype changed the internal seed/name maps to the existing fast hasher
and replaced `contains_key` followed by `get` with a single borrowed lookup on
hits. It passed **267 library tests**, including direct-noise bit comparisons for
all registered names, table growth and extreme seeds. The benchmark preserved
every block, ordered-mark and WG-map fingerprint.

The alternating comparison was mixed:

| Workers | Phase 3 chunks/s | Prototype chunks/s | Throughput ratio |
|---|---:|---:|---:|
| 1 | 8.51 | 9.01 | 1.060× |
| 2 | 19.68 | 15.75 | 0.800× |
| 4 | 31.41 | 32.51 | 1.035× |

The small one/four-worker improvements do not establish a consistent gain in
the presence of the two-worker regression and host variation. This prototype
and its extra test were also removed; the production Rust tree remains the
published phase-3 version. No additional speedup is claimed.

A separate experimental native wave passed **91 requests / 15 histories** with
zero blocks/biomes/order/structures/marks/scored-light differences and the known
36 BE fields / 70 WG-presence differences. Its source-equality audit ran before
the prototype was removed. Experimental replay binary:
`a25773998c63929fe06d94479471aa86f294b7af0f6c17ae7a515d31b4c3a8dc`;
source archive: `be01345c32138a355289ae56d27833ac61d0f8b629ff4123a1142a56619cdd27`.
These are experimental identities, distinct from the accepted checkpoint's.

[Full native/BCore measurement](metrics/noise-fill-wg-v2-noise-lookup-2026-10-09.json)
and [alternating frozen comparison](metrics/noise-fill-wg-v2-noise-lookup-paired-2026-10-09.json).
Local audit: `target/full-parity-20261003/noise-lookup-audit-20261009.json`.

## Next measurement

Collect a fresh profile of the accepted phase-3 code. The earlier profile was
collected before three optimizations and cannot establish the current dominant
cost. Read-only native research selected `skipSamplingAboveY` as the next isolated
candidate: it can remove entire high-Y aquifer searches, each currently involving
12 center lookups. Its preliminary-surface prepass also adds work, so a net gain
must be measured with setup inside the timed boundary.

For a normal chunk origin `(bx,bz)`, retained native bytecode gives a maximum `H`
over an inclusive, step-four scan from `(bx-16,bz-16)` through `(bx+24,bz+24)`,
then `T = 12 * (floorDiv(H + 20, 12) + 1) + 10`. Native shortcuts only **after the
positive-density check**, at strictly `y > T`, returning the global fluid picker
with the update flag cleared. It does not skip density or Beardifier evaluation.
The 121-point prepass includes points outside the FlatCache grid; preserve the
existing bounds, evaluation mode and delegation there. Narrow base-height and
carver callers need separate geometry review. Native witnesses at `T-1/T/T+1`,
negative coordinates, surface extremes and update-flag transitions must precede
acceptance. This is a research specification, not an implemented optimization.

## Reproduce an alternating frozen comparison

Both inputs must be completed matched-output v2 runs with the same conditions.
The original frozen binary, source archive and manifest must still exist. The
helper verifies their hashes and launches private copies of the two binaries:

```powershell
python scripts/benchmarks/compare_frozen.py --before target/previous-run/results.json --after target/candidate-run/results.json --output target/new-paired-run
```

Use the heavy-work queue when active. The output directory must be new; request
files, complete samples, logs and binary identities are retained. Source archives
for historical pruned runs cannot be reconstructed by reusing their names.
