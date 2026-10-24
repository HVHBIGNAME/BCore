# Current 26.2 CPU profile — 2026-10-24

The fresh owned-child diagnostic collected **4,002 statistical leaf samples** and
matched the authenticated 26.2 baseline's complete block, ordered-mark and both
WG-heightmap fingerprints. The child exited **0**. The
[versioned profile summary](metrics/noise-cpu-profile-26_2-2026-10-24.json) preserves
the method, request, filtering/errors, complete symbol summary and artifact hashes.

## Observations

| Resolved leaf symbol | Samples |
| --- | ---: |
| `density::chunk::CellInterpolator::sample` | 645 |
| `noise_perlin::ImprovedNoise::noise` | 596 |
| `density::DensityFunction::evaluate` | 551 |
| `density::chunk::RootDensity::evaluate` | 418 |
| `aquifer::Aquifer::substance` | 211 |
| Density `cache_once` thread-local closure | 190 |

PDB source attribution gives **1,063** samples in `density/chunk.rs`, **732** in
`noise_perlin.rs`, **645** in `density.rs`, and **232** in `aquifer.rs`. **351**
samples have no source-file attribution. Source-line and symbol summaries are
different views of the same samples; their counts must not be added together.

These observations prioritize interpolation and noise/density evaluation for the
next isolated experiment. `CellInterpolator::sample` retains an indexed validity
check, lazy eight-corner evaluation and ordered interpolation. Its combined leaf
count does not distinguish hit/miss costs or prove that the validity check is the
dominant expense. The previous rejected cell-miss, constant-operand and Perlin
rounding experiments remain rejected.

## Method and boundaries

- Only the driver-owned AMD64 child is sampled using Windows
  `SuspendThread`, `GetThreadContext`, and `ResumeThread`, with local PDB line tables.
- The same **489-input** source snapshot as the final frozen release is built with
  release optimization and the explicit diagnostic override
  `CARGO_PROFILE_RELEASE_DEBUG=1`. This is a separate diagnostic binary from the
  default-release throughput executable.
- The request retains the current seed, eight chunks and one-worker budget, with
  ten warmup and **100 diagnostic repetitions** to collect samples. These repeated,
  sampled durations are excluded from the published 18-process throughput baseline.
- Sampling lasts about **23.51 seconds**, starts after a two-second settle, and uses
  a 3 ms loop delay plus a thread-activity heuristic of at least 50,000 cycles.
- The trace includes warmup, output fingerprinting and other process work. It has
  no inline call stacks, exact function durations or inclusive CPU-cost attribution.
- The sampler filtered **3,600** low-activity observations and recorded **4**
  `SuspendThread:5` failures. Those retained failures and unresolved samples limit
  attribution; they are not discarded from the published record.
- No host profiling policy was changed. Source, request, executable, PDB and sampler
  identities are checked; the profiled output matches the native baseline.

## Retained identities

| Artifact | SHA-256 |
| --- | --- |
| Raw profile | `38f82c04beb274d9eef223ff1e0e8932bc60a2cf919076562d215eb379d694a7` |
| Diagnostic executable | `0dab7fe43775f4ca12b3a0ec55579f3976b5c335bbdf0eb9ca289d7304faeec5` |
| PDB | `7646d4c18331e7a64250578a9052b252c9259926834191bfb1619f22e484bf64` |
| Source archive | `9c371cc33c871a23fffae5c8895171f33a63c1169848c8e300fdfaecc48aa0dd` |
| Native-matched baseline result | `445993fa5be405614d139122672459bb9dccf1cbf7c7e6d3ac3f17e25f66816c` |

Local raw samples, source/PDB/binary, request and frozen driver/sampler are retained
under `target/migration-26_2-cpu-profile-20261024-01/`. Capture job:
`migration-26_2-cpu-profile-20261024-01-1792793884653133600` (exit 0).
The summary publication separately rechecks artifact hashes, symbol-count totals
and the native fingerprints.

Further optimization requires exact output checks and repeated alternating frozen
comparisons under the [current throughput contract](performance.md). This profile
itself establishes no production speedup or cross-version performance gain.
