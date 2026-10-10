# BCore / Vanilla performance measurements

The current comparison measures **NOISE terrain material filling**, not a running
Minecraft server or the complete world-generation pipeline. Both implementations
produce the same complete block-state arrays, ordered per-section fluid
postprocessing marks **and both worldgen heightmaps** for every measured and
warmup batch.

![Measured NOISE throughput](../site/assets/performance.svg)

The chart remains the October 8 measurement of the accepted implementation.
Two later [October 9 cache experiments](performance-experiments-2026-10-09.md)
were removed after negative or mixed alternating-binary measurements. Their full
statistics are retained; neither is counted as a production speedup.

## Results — 2026-10-08, contract `noise-fill-wg-v2`

Eight fresh chunks per batch; median of three independent process medians. Each
process runs ten warmup batches and ten timed batches. Higher throughput is better.

| Worker budget | Vanilla 26.1 batch | BCore release batch | Vanilla chunks/s | BCore chunks/s |
|---|---:|---:|---:|---:|
| 1 | 120.91 ms | 660.20 ms | 66.16 | 12.12 |
| 2 | 104.93 ms | 493.39 ms | 76.24 | 16.21 |
| 4 | 39.45 ms | 181.25 ms | 202.77 | 44.14 |

**BCore is still slower in this bounded kernel workload**, with a measured gap of
about **4.6×–5.5×**. BCore's one-to-four-worker throughput now rises about
**3.64×**; Vanilla's rises about **3.07×**. These are
local observations on an unisolated machine, not a server TPS ranking.

### What the optimization series changed

Three exact-behavior passes were measured under this identical contract, each
verified against the same output fingerprints (all blocks, ordered marks and both
WG maps):

| Pass | Change | BCore chunks/s, 1 / 2 / 4 workers |
|---|---|---:|
| Baseline `3eb15d3` | — | 2.04 / 3.24 / 4.31 |
| 1 | Aquifer centers keyed instead of linearly scanned, quart-quantized preliminary surface heights memoized (measured 140.9× redundancy), one aquifer/material context per Rayon job | 4.04 / 6.39 / 8.35 |
| 2 | Fast deterministic hasher for internal lookup tables whose keys are exact bits, and seeded normal-noise lookups by borrowed name (two `String` allocations per sample removed) | 11.30 / 12.60 / 15.50 |
| 3 | Material/density cache lifetime moved to the whole chunk, matching native's one `NoiseChunk`/aquifer per chunk; nested column parallelism removed because callers already parallelize across chunks | 12.12 / 16.21 / 44.14 |

Cumulative gain at four workers is **10.24×** over the first baseline of this
contract (**4.3096 → 44.1368 chunks/s**). The repeated baseline measured
**4.9705 chunks/s**, giving **8.88×** against that repeat. The
[compact series](metrics/noise-fill-wg-v2-series-2026-10-08.json) preserves both
baselines and all three passes, with per-process medians and raw-result hashes.
Profiling that motivated the passes measured, per 24 chunks: 601,890 preliminary
surface evaluations over only 1,424 distinct coordinates, ~13.9M `FindTopSurface`
steps beneath them, and Rayon-dependent cache fragmentation (48 jobs at one worker
versus 385 at four, with 2D-cache misses rising 4.7×).

### Honest caveats

- Across these runs on the same machine, one-worker Java run summaries ranged
  **26.3–66.2 chunks/s**.
  BCore's one-worker baseline summaries were **2.04 and 2.06 chunks/s**, but its
  four-worker baselines varied by about **15%**. Both engines remain subject to
  machine noise; the series preserves process-level variation rather than a
  confidence interval or an isolated causal estimate. JIT/GC or scheduling may
  contribute, but this series did not profile the cause of the variation.
- The timed contract is new. It includes Rust's normal post-NOISE WG heightmap
  capture (which the earlier October 5 measurement omitted) and keeps Rust's
  surface-biome bookkeeping and native reflection/scratch work inside the timer.
  It is therefore **not** comparable with the earlier `noise-fill-kernel` series.
- Every benchmark pass matches the bounded kernel fingerprints. Separately, the
  [final 665-test checkpoint](metrics/checkpoint-2026-10-08-optimized.json)
  records 88 requests with zero differences in 8,650,752 block-state observations,
  135,168 biome-cell observations and 40 scored light snapshots. It still has
  **36 block-entity field differences and 70 WG-presence differences**; none of
  those requests has complete native coverage. This is not full parity.

[Final phase-3 samples, fingerprints and provenance](metrics/noise-fill-wg-v2-2026-10-08.json)

## Previous measurement — 2026-10-05, contract `noise-fill-kernel`

Kept unchanged and published separately; its timed boundary did not include the
WG heightmap capture.

| Worker budget | Vanilla 26.1 batch | BCore release batch | Vanilla chunks/s | BCore chunks/s |
|---|---:|---:|---:|---:|
| 1 | 162.70 ms | 3,028.97 ms | 49.17 | 2.64 |
| 2 | 93.87 ms | 2,179.34 ms | 85.23 | 3.67 |
| 4 | 47.48 ms | 1,373.06 ms | 168.49 | 5.83 |

At four workers the measured difference was about **28.9×** under that contract.
All eighteen processes and 180 batches matched in block states and ordered marks,
including warmups, and the machine was not OS-isolated.

[October 5 samples, fingerprints and provenance](metrics/noise-fill-2026-10-05.json)

## Workload and controls

| Setting | Value |
|---|---|
| CPU | AMD Ryzen 7 5700X, 8 physical cores / 16 logical processors |
| OS | Windows 11, build 26200 |
| Native engine | Original Minecraft Java 26.1 / protocol 775 JAR |
| Rust build | Release, `--locked`, diagnostics-enabled benchmark wrapper |
| World seed | `846692123413862008` |
| Chunks | `(0,0)`, `(1,0)`, `(0,1)`, `(1,1)`, `(-64,-32)`, `(-63,-32)`, `(62,0)`, `(-125,187)` |
| Geometry | 16 × 384 × 16 per chunk, Y=-64..319 |
| Inputs | Fresh empty chunks, original overworld noise settings, empty structure references, no blending |
| Included work | Production material fill, aquifers, noise ore veins, fluid postprocessing requests and both WG heightmaps |
| BCore parallelism | Explicit Rayon pool of 1 / 2 / 4 workers; independent chunks submitted as a batch |
| Vanilla parallelism | `max.bg.threads=N`, `ActiveProcessorCount=N+1`; original async `fillFromNoise` tasks, verified as a `ForkJoinPool` with that parallelism |
| Run order | Fresh processes run serially; engine order alternates between repetitions |
| Verification | SHA-256 of all block states, ordered per-section marks and both 256-entry WG maps, checked across batches, engines and worker counts |

A worker budget is not an OS CPU-affinity cap. Java runtime/GC and the caller
threads may perform additional work. Native task bodies are executed unchanged;
the harness uses reflection only to construct inputs, invoke the entry point and
inspect outputs.

The two engines keep their own production storage and kernel implementation.
**Compilation, registry/JVM startup, warmup batches, initial chunk allocation,
hashing, BIOMES, structure generation, SURFACE, CARVERS, FEATURES, LIGHT, SPAWN,
FULL, packet encoding, disk I/O and gameplay are excluded.**

The eight chunks contain **786,432 block positions** and **284 postprocessing
marks**. Repeated observations do not increase the number of unique tested world
positions.

Paper, Purpur and SteelMC were not run in this benchmark; there are no invented
bars or estimates for them. Complete-server comparisons need a separate common
workload once the relevant runtime features are integrated.

## Reproduce

Requirements: the repository's Rust toolchain, Python 3.10+, `javac` 21+, Java 25
and the pinned 26.1 server JAR with its runtime libraries. Native asset/capture
setup is described by the [native-history harness](../scripts/native-generation-reference/README.md).
The runner verifies the original server JAR digest before measuring and refuses a
project-local Cargo configuration:

```text
a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52
```

From the workspace root, choose a **new** output directory:

```powershell
python scripts/benchmarks/run.py --output target/noise-bench-new --label local --workers 1 2 4 --processes 3 --warmup 10 --batches 10
```

Every run archives the complete build input set, builds from that snapshot, and
launches a frozen read-only copy of the executable, re-verifying artifact hashes
before and after each measured process. `--compare-to <results.json>` refuses to
compare runs whose contract, harness bytes, workload, toolchain, native classes,
libraries or environment differ, which is how the baseline/optimized pairs above
were produced. Benchmark and full test processes must run serially.

The runner preserves logs, source hashes, all samples and `results.json`. A block,
mark or WG-map mismatch fails the run before publishing a matched-output summary.
Source changes during measurement also fail verification.

The published `noise-fill-kernel` run is `noise-benchmark-1791200017542476800`. The
earlier `noise-benchmark-1791199670262415500` was a failed harness
attempt: Minecraft prefixes stdout with its logger, and the initial result parser
expected the marker at column zero. That attempt is not included in the plotted
samples. The `noise-fill-wg-v2` series contains
`noise-fill-wg-v2-baseline-final-01`, `noise-fill-wg-v2-optimized-final-01` (pass 1),
`noise-fill-wg-v2-phase2-final-01` and `noise-fill-wg-v2-phase3-final-01`, plus
`noise-fill-wg-v2-baseline-repeat-02` to assess machine noise. Exact local paths,
raw-result SHA-256 hashes and recorded source/binary identities are in the
[compact series](metrics/noise-fill-wg-v2-series-2026-10-08.json). Historical local
snapshots may be pruned; these hashes identify evidence but are not a downloadable
replay bundle. Rebuilding the current tree does not recreate an earlier snapshot.

## Reading other performance numbers

Earlier [density-cache measurements](parity-report.md#density-cache-measurements)
compare previous BCore implementations and include explicitly recorded contention.
They are not mixed into this fresh-process Vanilla comparison. Test-suite duration
is likewise not treated as world-generation throughput.
