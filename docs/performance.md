# BCore / Vanilla performance measurements

The current comparison measures **NOISE terrain material filling**, not a running
Minecraft server or the complete world-generation pipeline. Both implementations
produce the same complete block-state arrays and ordered per-section fluid
postprocessing marks for every measured and warmup batch.

![Measured NOISE throughput](../site/assets/performance.svg)

## Results — 2026-10-05

Eight fresh chunks per batch; median of three independent process medians. Each
process runs five warmup batches and five timed batches. Higher throughput is better.

| Worker budget | Vanilla 26.1 batch | BCore release batch | Vanilla chunks/s | BCore chunks/s |
|---|---:|---:|---:|---:|
| 1 | 162.70 ms | 3,028.97 ms | 49.17 | 2.64 |
| 2 | 93.87 ms | 2,179.34 ms | 85.23 | 3.67 |
| 4 | 47.48 ms | 1,373.06 ms | 168.49 | 5.83 |

**BCore is slower in this workload.** At four workers, the measured difference is
about **28.9×**. BCore's one-to-four-worker throughput increases about **2.21×**;
Vanilla's increases about **3.43×**. These results identify an optimization target,
not a general verdict on Rust versus Java or on server TPS.

The plotted whiskers span the three process-median results. For example, the
four-worker batch medians range from **39.36–51.06 ms for Vanilla** and
**1,365.98–1,490.15 ms for BCore**. The machine was not OS-isolated, so these are
local observations with visible variability, not confidence intervals or a
cross-hardware ranking.

[All samples, fingerprints and provenance](metrics/noise-fill-2026-10-05.json)

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
| Included work | Production material fill, aquifers, noise ore veins and fluid postprocessing requests |
| BCore parallelism | Explicit Rayon pool of 1 / 2 / 4 workers; independent chunks submitted as a batch |
| Vanilla parallelism | `max.bg.threads=N`, `ActiveProcessorCount=N+1`; original async `fillFromNoise` tasks |
| Run order | Fresh processes run serially; engine order alternates between repetitions |
| Verification | SHA-256 of all block states and ordered per-section marks, checked across batches, engines and worker counts |

A worker budget is not an OS CPU-affinity cap. Java runtime/GC and the caller
threads may perform additional work. Native task bodies are executed unchanged;
the harness uses reflection only to construct inputs, invoke the entry point and
inspect outputs **outside** the timed section.

The two engines keep their own production storage and kernel implementation.
BCore's internal terrain bookkeeping is included in its existing fill method.
**Compilation, registry/JVM startup, warmup batches, initial chunk allocation,
hashing, BIOMES, structure generation, SURFACE, CARVERS, FEATURES, LIGHT, SPAWN,
FULL, packet encoding, disk I/O and gameplay are excluded.**

The eight chunks contain **786,432 block positions** and **284 postprocessing
marks**. Full fingerprints match in all 18 processes / 180 batches, including
warmups. Repeated observations do not increase the number of unique tested
world positions.

Paper, Purpur and SteelMC were not run in this benchmark; there are no invented
bars or estimates for them. Complete-server comparisons need a separate common
workload once the relevant runtime features are integrated.

## Reproduce

Requirements: the repository's Rust toolchain, Python 3.10+, `javac` 21+, Java 25
and the pinned 26.1 server JAR with its runtime libraries. Native asset/capture
setup is described by the [native-history harness](../scripts/native-generation-reference/README.md).
The runner verifies the original server JAR digest before measuring:

```text
a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52
```

From the workspace root, choose a **new** output directory:

```powershell
python scripts/benchmarks/run.py --output target/noise-bench-new --workers 1 2 4 --processes 3 --warmup 5 --batches 5
```

`--java` and `--target-dir` select different local runtime/build paths. With an
already-built matching executable, `--skip-build` skips compilation. Benchmark
and full test processes should run serially on the measurement machine.

The runner preserves logs, source hashes, all samples and `results.json`. A block
or postprocessing mismatch fails the run before publishing a matched-output
summary. Source changes during measurement also fail verification.

The published run is `noise-benchmark-1791200017542476800`. The earlier
`noise-benchmark-1791199670262415500` is preserved as a failed harness attempt:
Minecraft prefixes stdout with its logger, and the initial result parser expected
the marker at column zero. That attempt is not included in the plotted samples.

## Reading other performance numbers

Earlier [density-cache measurements](parity-report.md#density-cache-measurements)
compare previous BCore implementations and include explicitly recorded contention.
They are not mixed into this fresh-process Vanilla comparison. Test-suite duration
is likewise not treated as world-generation throughput.
