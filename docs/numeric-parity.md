# Numeric parity lab (26.1)

`math_lab` is the fast inner loop for worldgen changes: identical inputs go to
the real Java implementation and the Rust kernel; outputs are compared by their
IEEE-754 bits. The checked-in capture needs no running server or JRE to test.

## Quick start

From the repository root, with `BCORE_DATAPACK` unset:

```powershell
cargo build --release --locked -p bcore-worldgen --example math_lab
$lab = ".\target\release\examples\math_lab.exe"
$reference = "crates/bcore-worldgen/data/numeric_26_1.json"
& $lab check $reference
& $lab check $reference --filter density/overworld/factor/
& $lab bench $reference --filter normal/erosion/ --iterations 3000 --rounds 7
& $lab experiment $reference --filter scalar/ --iterations 10000 --rounds 7
```

Add `--json` for machine-readable reports. `check` exits **1** on any bit
difference and **2** on invalid input. `bench` first requires a passing reference
check. `experiment` reports each candidate's `bit_exact`/mismatch count and timing;
rejected candidates are an expected experiment result, not a failing baseline.

For the regression suite, including reversed inputs and four worker threads:

```powershell
cargo test --release --locked -p bcore-worldgen --test numeric_reference
```

## Native reference and coverage

`scripts/NumericReference.java` invokes native methods using exact reflection
signatures. `RandomState` wires density functions and their noises. The native
`NoiseChunk` path initializes its lattice, fills a cell and samples its actual
cached density. It uses the empty Blender and zero beardifier. These probes need
neither block chunks nor gameplay ticks.

Pinned JAR: `target/vanilla-775/versions/26.1/server-26.1.jar`, SHA-256
`a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`.
The fixture includes the source hash, every input, operation/configuration and
expected result. Floats use 8-digit hex, doubles 16-digit hex. JSON parsing keeps
full signed 64-bit seeds and correctly rounded doubles (`float_roundtrip`).

| Kernel | Cases | Values |
|---|---:|---:|
| Smoothstep, wrap, f32/f64 lerp, clamped lerp | 5 | 1,135 |
| ImprovedNoise, including the float-epsilon boundary | 18 | 1,350 |
| PerlinNoise, sparse/positive/16-octave configurations | 24 | 1,800 |
| All 62 NormalNoise parameter sets, with overworld Xoroshiro seeding | 372 | 27,900 |
| BlendedNoise, two configurations | 12 | 768 |
| Raw density nodes, rarity boundaries, splines, gradients and configuration-cache checks | 338 | 21,632 |
| All 15 overworld router fields, raw | 90 | 5,760 |
| Native NoiseChunk final density and five marker wrappers | 36 | 2,304 |
| **Total** | **895** | **62,649** |

Seeds: `0`, `1`, `-1`, `846692123413862008`, `i64::MIN`, `i64::MAX`.
Inputs include negative coordinates, cell/chunk boundaries, Y=-64..319, positions
near the world border, sparse octaves, threshold neighbours and signed zeros.

Recreate and independently verify the standard capture (javac 21+, Java 25+):

```powershell
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe numeric --build-dir target/native-probes/numeric-standard --output target/numeric-verified.json --verify crates/bcore-worldgen/data/numeric_26_1.json
```

`scripts/numeric_cases.py` defines the input matrix. Expected outputs are always
obtained from the JAR. Changes to Java probe sources require a fresh capture;
`--verify` checks source/JAR provenance as well as values.

All native probes share the capture runner's `--build-dir` option. Use a distinct
directory and `--output` path for each concurrent invocation so Java helper class
files and generated requests cannot overwrite another probe's files. Omitting
`--build-dir` uses the shared `target/tree-reference` directory for sequential work.

## Your own inputs and functions

`scripts/numeric-example.json` is a runnable request containing custom points,
seeds, Perlin amplitudes, float/double interpolation and an inline density graph.

```powershell
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe numeric --build-dir target/native-probes/numeric-custom --input scripts/numeric-example.json --output target/numeric-custom.json
& $lab check target/numeric-custom.json
& $lab sample scripts/numeric-example.json
```

Requests have `points: {name: [[x,y,z], ...]}` and a `cases` array. Each case has
an `id`, `points` group and `op`. The operations and required fields are visible in
`examples/support/numeric.rs::Spec`:

- `smoothstep` / `wrap`: use the first coordinate.
- `lerp`: `[t,a,b]`, with `precision: "f32"` or `"f64"`.
- `clamped_lerp`: `[t,a,b]` in f64.
- `improved`: `seed`, `y_scale`, `y_fudge`.
- `perlin`: `seed`, `first_octave`, `amplitudes` (modern initialization).
- `normal`: `seed`, registered `noise` identifier.
- `blended`: `seed`, `xz_scale`, `y_scale`, `xz_factor`, `y_factor`, `smear`.
- `density`: `seed`, `function` (identifier or native density JSON), raw evaluation.
- `router`: `seed`, `field` from overworld `noise_router`, raw evaluation.
- `noise_chunk`: `seed`, optional `function` override; defaults to final density.

Density/BlendedNoise coordinates are integer block positions. `noise_chunk`
uses overworld 4×8×4 cells and requires Y=-64..319. Raw density markers delegate
to their inputs; wrapped NoiseChunk markers perform interpolation/caching. Their
outputs are deliberately separate test cases. The final-density cell cache is
filled with native **X/Y/Z** trilinear rounding; the subsequent native
`updateForY/X/Z` path is a different evaluation order.

`sample` emits Rust result bits and removes native provenance. To add a new
oracle-backed baseline, run the Java capture on the same request.

## Two formulas for experiments

Candidates live in `crates/bcore-worldgen/examples/support/formulas.rs`.
Add a named candidate there and rerun `experiment`; the production implementation
is selected separately by `Kernel` in `support/numeric.rs`.

### 1. Perlin fade

Mathematically: **f(t) = 6t⁵ − 15t⁴ + 10t³**. Native f64 evaluation order:

```rust
t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
```

The provided expanded-polynomial candidate differs on **174/268** inputs.
The FMA candidate differs on **2/268**, including a neighbour of 1.0. Both fail
the exactness gate, even though the algebra is equivalent.

### 2. Linear interpolation

Mathematically: **L(t,a,b) = a + t(b − a)**. Native order:

```rust
a + t * (b - a)
```

For f64, `(1-t)*a + t*b` differs on **109/264** inputs and
`t.mul_add(b-a, a)` on **86/264**. The f32 candidates differ on **109/264** and
**84/264**, respectively. Example: `t=1, a=1e16, b=1` gives native `0`, while the
weighted expression gives `1`. Signed zero is also checked.

`f32_output_changes` counts native double results altered by rounding the output
to float and back. It is a precision diagnostic, not a benchmark of an all-f32
noise implementation. Minecraft deliberately mixes precisions: CubicSpline uses
float intermediates while ImprovedNoise/PerlinNoise use doubles.

## Measured corrections and optimization — 2026-09-25

The first **851-case / 59,633-value** matrix exposed **2,570 bit differences**.
Float-accurate splines and the native ImprovedNoise epsilon corrected those
differences; the expanded matrix also covers the cache/configuration and gradient
fixes below:

- CubicSpline's float coordinates, knots, derivatives, nested values and native
  two-lerp polynomial; exact interval and zero-derivative extension rules.
- ImprovedNoise's `1e-7f` epsilon widened to double, rather than a double literal.
- Raw marker evaluation separated from NoiseChunk semantics.
- CacheAllInCell keeps individual block values; FlatCache uses its bounded quart
  grid at Y=0 and direct evaluation outside the grid.
- BlendedNoise caches include every configuration parameter, not only the seed.
- Clamped interpolation and Y-clamped gradients retain native endpoint behavior.

The expanded **62,649-value** matrix passes with **zero bit differences**. A local
release check took **0.42 s** including kernel construction, excluding JSON loading
and process startup. Four regression tests, including reversed/parallel evaluation
and deliberately corrupted reference bits, passed in **0.48 s**.

The full worldgen regression run at this numerical milestone had **123 passed,
2 ignored**. These are recorded results for that phase, not a total for subsequent
entity-streaming or isolated feature work.

Perlin's starting frequency/value factors are now computed once in the
constructor. The sampling loop retains its native operation order. Local
`normal/erosion/` measurements, Windows x86-64, release, six seeds, 75 inputs per
seed, 3,000 repetitions and seven rounds:

| Seed | Before, median ns/value | After, median ns/value |
|---|---:|---:|
| 0 | 378.50 | 213.58 |
| 1 | 364.00 | 199.13 |
| -1 | 362.98 | 197.04 |
| 846692123413862008 | 364.88 | 194.47 |
| i64::MIN | 360.79 | 209.52 |
| i64::MAX | 362.87 | 205.82 |

This is approximately **1.8×** for that kernel on this machine. Benchmarks warm
the kernel, use `black_box`, and exclude JSON, construction and comparison. JSON
output includes each round and min/median/max. Keep before/after reports with the
same compiler/CPU settings; these timings do not measure complete chunk generation.

## Shared native Mth lookup

`scripts/MthReference.java` captures the actual 26.1 `Mth` table and method results
in `crates/bcore-worldgen/data/mth_26_1.json`. Verified coverage:

- All **65,536 native f32 table entries** match bit for bit.
- **1,903 angle cases** comprise **898 f32-promoted inputs** and **1,005 f64
  inputs**, producing **3,806 sine/cosine outputs with zero bit mismatches**.
  Inputs include signed zeros, NaNs, infinities, saturation, overflow and
  table-index boundaries.
- **Three Rust reference tests passed**, including an **eight-thread** test that
  checks shared table identity and native result bits in reversed input order.

The SHA-256 of the complete table's **little-endian f32 bits**, in index order, is
`cdfaec6870788e193dff3f1ddfa3ca7d3f1a897042d78365a1fe3a433be3627d`.
The fixture's `sin_table_sha256_le` and the Rust full-table test check this digest.

The native signatures are **`Mth.sin(double) -> float`** and
**`Mth.cos(double) -> float`**. There are no native float-argument overloads in
26.1: an f32 argument widens to f64 **before** scaling. The Rust `sin_f32` and
`cos_f32` helpers preserve that promotion. Indexing uses Java-style saturating
double-to-long conversion, then the low 16 bits; cosine adds its phase before
conversion. NaN converts to a zero index before masking.

`crates/bcore-worldgen/src/mth.rs` initializes one heap-backed **256 KiB** table
through `OnceLock`. Carvers and the ore-radius Mth calls now use this shared
lookup. At the lookup-integration stage, **10 carver tests, 3 Mth tests and 2 ore
tests passed**. These are targeted results, separate from the historical
**62,649-value / 895-case** numeric lab matrix and the
[recorded regression suites](parity-report.md#regression-suites).

### Per-call microbenchmark

Measured on **AMD Ryzen 7 5700X, Windows, release build**, using **16,384 inputs
per kernel, 256 iterations and nine rounds**, with alternating baseline/lookup
measurement order. The inputs are deterministic finite angles in `[-8π, 8π)`.
The baseline is the previous indexed computation: derive the native table index,
compute that entry's sine dynamically and round to f32. Both sine and cosine
lookup paths are compared against this baseline, with each benchmark input checked
for exact agreement.

| Call | Dynamic computation, median ns/call | Shared lookup, median ns/call | Speedup |
|---|---:|---:|---:|
| Sine, f64 input | 15.663 | 1.236 | 12.67× |
| Cosine, f64 input | 16.110 | 1.447 | 11.14× |
| Sine, f32 input promoted to f64 | 15.544 | 1.366 | 11.38× |
| Cosine, f32 input promoted to f64 | 16.229 | 1.370 | 11.84× |

Cold table initialization took **385.2 µs**, measured separately. The runner warms
the kernels, uses `black_box`, alternates measurement order and reports individual
rounds and medians. These are **per-call timings**, not whole-chunk, full carver or
world-generation speedups. The earlier **~1.8× erosion-noise** result remains a
separate Perlin optimization measurement.

Reproduce from the repository root; native recapture uses its own build directory
so it can run alongside other Java probes:

```powershell
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe mth --build-dir target/native-probes/mth --output target/mth-reference.json --verify crates/bcore-worldgen/data/mth_26_1.json
cargo test --release --locked -p bcore-worldgen --test mth_reference
cargo test --release --locked -p bcore-worldgen --lib carver::
cargo test --release --locked -p bcore-worldgen --test ore_reference
cargo run --release --locked -p bcore-worldgen --example mth_bench -- --iterations 256 --rounds 9
```

The benchmark emits JSON containing its input count, warmup passes, rounds,
initialization time and per-call measurements. Each concurrent native capture
still needs a unique `--build-dir` and `--output` path.

Numerical coverage is a component milestone. Generated chest-minecart streaming
now includes chunk-batch ordering, shared runtime identities and unload/teleport
removal; see [entity-streaming.md](entity-streaming.md) for its verification and
immutable pre-gameplay scope. [parity-report.md](parity-report.md) records the
completed world-specific queue, real carver surface resolver (**1,431 native
cases; 13 tests passed**) and region-aware standing trees (**438 native cases**).
Owning-chunk [tree-effect transfer and `.bcc` v4 persistence](tree-effects.md) are
implemented, alongside tested chunk-local tick containers. Global region-vegetation
scheduling and runtime tick execution remain pending. The three-region snapshot
and its executable provenance are recorded in the parity report.

Full region scheduling, remaining features and structures, other dimensions,
minecart motion, loot/inventory interaction and spawner simulation remain tracked in
[worldgen-parity-plan.md](worldgen-parity-plan.md).
