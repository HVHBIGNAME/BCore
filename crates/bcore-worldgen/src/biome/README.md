# Native 26.1 climate lookup

`ParameterList` implements the target JAR's seven-dimensional `Climate.RTree`.
The implementation is MIT-licensed and independently written around a contiguous
node arena. `BiomeTreeReference.java` invokes the native implementation directly.

## Generation integration

Keep one immutable list for the lifetime of the native biome-source context:

```rust
use bcore_worldgen::biome::{self, ParameterList};

let parameters = ParameterList::overworld();
let result = biome::biome_at(&parameters, temperature, humidity,
    continentalness, erosion, depth, weirdness);
```

For a configured/datapack list, use
`ParameterList::new(biome::parse_parameters(&json)?)`. The `VanillaGraph` hook is
to change its `parameters` field from `Vec<...>` to `biome::ParameterList` and
wrap its existing parser result in `ParameterList::new`. Its `biome_at` call can
stay as written. The current main-owned graph integration stores this type,
calls `parameters.find(climate_at(...))`, loads the canonical rows by default,
and gives each fork a fresh list/cache identity.

Public entry points:

- `biome_at(&source, t, h, c, e, d, w)` preserves the call shape and quantization.
- `ParameterList::find([f64; 6])` uses native per-thread history.
- `BiomeLookup::find_biome([i64; 6])` accepts an already-quantized target.
- `ParameterList::sampler()` creates an explicit, borrowed `BiomeSampler` with
  `find`, `find_quantized`, and `reset`. Keep it across the entire intended stream.
- `ParameterList::reset_thread_cache()` explicitly resets only the calling thread.
- `values()`, `node_count()`, and `storage_bytes()` support inspection/measurement.

**Bare slices, arrays and vectors have cold-search semantics.** Rust cannot prove
that such an argument is the same live, immutable list as on a previous call.
Those compatibility calls validate all rows against a single memoized tree per
thread, then search from an empty last-result cache. This is a native cold tree
search, not native persistent history, and validation remains O(n). An owning
`ParameterList` is required for the exact native cache and fast generation path.
`Arc`, `Box`, and reference wrappers forward to their underlying source.

`ParameterList::overworld()` reads the JAR's captured **integer** parameters,
preserving row order without a float codec round trip. It translates names into
BCore's existing wire registry. Native 26.1 has **65** registry entries; BCore's
wire map has **66**, with **12** differing numeric IDs. Compare resource keys.
All 7,593 currently bundled parameter rows also match the captured rows exactly.

## Construction and search invariants

- Axis order is temperature, humidity, continentalness, erosion, depth,
  weirdness, offset. The target's offset is zero; branch offset bounds still
  participate in distance.
- There are at most six children. Small groups use a stable sort by the sum of
  absolute integer centers. Centers divide toward zero, including negative odds.
- Larger groups try all seven rotated lexicographic center orders, in place.
  The first strictly cheaper sum of bucket widths wins. Its bucket membership
  and order are snapshotted before trying later axes.
- Bucket size is the largest power of six strictly below the number of rows,
  equivalent to the native log/pow formula for positive Java list lengths.
  The chosen buckets are stably ordered by absolute centers before recursion.
- Bounds are inclusive. Distance uses native wrapping `long` arithmetic,
  including the above-range branch's precedence when subtraction overflows.
- Traversal retains child order. Both pruning and replacement are strict.
  Equal distance retains the incumbent leaf, including the previous result.
  There is no extra row-ID, biome-ID, or resource-name tie policy.

Each owning list has its own weakly tracked identity. Even identical independently
constructed lists have separate histories. A thread keeps one leaf index per live
list it has queried; dead identities are pruned on insertion and sparse capacity
is reduced. Trees are never retained by thread-cache entries. Storage is bounded
by live lists and threads, with no growth per coordinate, seed, or query. A fixed
LRU eviction limit on **live** histories would change native tie behavior.

## Oracle and coverage

Pinned server JAR:
`target/vanilla-775/versions/26.1/server-26.1.jar`

SHA-256:
`a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`

The only probe source dependency is `scripts/BiomeTreeReference.java`.
`source_hash_order` records that order; hashing normalizes CRLF to LF before
concatenation. Each runtime JAR dependency has its own path and SHA-256 in the
capture. The probe loads those JARs itself and does not depend on shared probe
helpers or capture scripts.

Checked-in data:

- `data/biome_parameters_26_1.json`: canonical ordered integer rows and provenance.
- `data/biome_tree_26_1.json`: 106,893 query inputs in 36 groups; cold, linear,
  forward, reverse, and shuffled native results; concurrent and context streams.

Verified coverage includes:

- All **20** topology hashes, including bounds and ordered children/leaf indices:
  17 main lists and three context-isolation lists.
- **748,251** exact Rust selections against native cold/TLS/explicit-stream
  expectations, plus slice compatibility checks and concurrent/context checks.
- **176,256** raw router doubles and **29,376** biome resource keys, across six
  seeds and overworld/large-biomes/amplified configurations.
- All **4,608** previously captured biome resource keys through the actual
  `VanillaGraph::noise_biome_at` path. The existing captured-terrain integration
  test and all **62,649** numeric values also pass.
- Seeds `0`, `1`, `-1`, `846692123413862008`, `i64::MIN`, and `i64::MAX`.
- Three full quart-biome chunk streams per overworld seed, all section heights,
  negative coordinates, world-border positions, and sparse router points.
- Every canonical row center, each distinct range endpoint and its neighbors,
  random climate points, offsets, overlapping/equidistant ranges, reversed rows,
  negative odd centers, and sizes around powers of six through 1,297 leaves.
- Native float quantization, signed zero, NaN/infinities, saturation, reversed
  direct-constructor ranges, wrapping subtraction/squares/sums, and one-leaf roots.
- Four simultaneous native/Rust thread streams and interleaved identical and
  differently configured lists. Rust lifetime checks retain 96 live warm contexts
  across 1,024 temporary lists, then verify dead cache storage is reclaimed.

The native matrix contains **184** linear-versus-forward row differences
(**164** resource-key differences), and **61** cold-versus-forward row differences.
These are measured native tie/history effects, not parity failures.

Read-only studies used:

- SteelMC `f72cf18c767e8c4eb4f92b5b2140ff51b5bf78f6`,
  `steel-utils/src/climate/{parameter_list,types}.rs` and
  `steel-worldgen/src/biomes/{biome_source,climate_sampler}.rs`.
  Its documented per-chunk history reset is deliberately not used here.
- Pumpkin `4426d1113a211e6018a2db416e33b6b8a7802614`,
  `tools/pumpkin-codegen/src/biome.rs` and the worldgen multi-noise sampler.
  Its extracted static tree and explicit previous-result interface were reviewed.
- PaperMC `abfdaed87a695450cdaa5711ec52833b3ce8997d`,
  `CraftBiomeParameterPoint.java`, and the local Mache `Climate.java.patch`.
  These inform API/units; the **26.1 JAR**, rather than another project's current
  Minecraft version, determines all captured expectations.

## Reproduce

From the repository root, with two Cargo jobs and the isolated target directory:

```powershell
$target = "$env:TEMP\opencode\bcore-biome-target"
$env:BCORE_DATAPACK = $null
cargo test --release --locked -p bcore-worldgen --lib biome:: -j 2 --target-dir $target -- --nocapture
cargo test --release --locked -p bcore-worldgen --test numeric_reference --test vanilla_reference -j 2 --target-dir $target
```

Compile the standalone oracle from the repository root:

```powershell
javac -d target/biome-tree-reference scripts/BiomeTreeReference.java
```

With working directory `target/biome-tree-reference`, run Java 25+:

```powershell
$repo = 'C:\coding\MINECRAFT\BCore'
& "$repo\target\jre25\jdk-25.0.4.1+1-jre\bin\java.exe" -Xmx2G -cp . BiomeTreeReference $repo verified.json verified-canonical.json "$repo\crates\bcore-worldgen\data\biome_tree_26_1.json"
```

The final argument requires equality with the checked-in capture, including
provenance. Both output files and native logs stay in the dedicated probe directory.
The self-contained Java probe exceeds the quality tool's generic 400-line style
limit; its reflection, input matrix, and capture code remain together to keep the
single-source dependency/provenance contract explicit.

### Benchmark

```powershell
$env:BCORE_BIOME_BENCH_OUTPUT = "$target\biome-benchmark.json"
cargo test --release --locked -p bcore-worldgen --lib biome::benchmark::benchmark_native_tree_against_original_linear_on_identical_router_inputs -j 2 --target-dir $target -- --ignored --exact --nocapture
```

Defaults: 28,224 native overworld router inputs from all six seeds, four repetitions,
seven rounds, forward/reverse order, and rotating measurement order. Override with
`BCORE_BIOME_BENCH_ITERATIONS` and `BCORE_BIOME_BENCH_ROUNDS`. Each seed's stream is
reset at the same boundary as its native oracle capture. The old linear scan and
the owning tree are separately checked against their corresponding native results
before timing. All implementations receive identical f64 input bits; the report
contains their SHA-256. `black_box` prevents elision. Construction, JSON parsing,
and parity checks are outside the timed lookups; construction is reported separately.
These are biome-selection measurements, not whole-chunk generation timings.

### Measured result — 2026-09-30

AMD Ryzen 7 5700X, Windows x86-64, release, Rust
`1.100.0-nightly (8925ea358 2026-08-20)`, LLVM 23.1.0. The command above completed
seven rounds with four repetitions and 28,224 identical input points:

| Lookup | Forward median, ns/query | Reverse median, ns/query |
|---|---:|---:|
| Original linear scan | 29,372.48 | 34,647.15 |
| Owning list, native TLS | 275.31 | 417.43 |
| Explicit borrowed stream | 269.80 | 418.01 |
| Cold slice compatibility | 22,765.69 | 31,291.69 |

Owning-list speedup: **106.69× forward / 83.00× reverse**. The explicit stream
measured **108.87× / 82.89×**. Construction took **12.5422 ms**, and 9,112 tree
nodes plus 7,593 rows occupy **1,943,856 bytes**, excluding allocator/bookkeeping.
Timing rounds were variable; all individual rounds are retained in
`%TEMP%/opencode/bcore-biome-target/biome-benchmark.json`.

The real router benchmark has zero old/new biome-resource-key differences in
either direction. Boundary and synthetic native cases separately demonstrate the
tie/history differences described above. Input SHA-256 (little-endian f64 bits,
forward seed-group order):
`272046b5f10c8d8365276e9eff3e236b80915518f953439d9184650490505bd1`.

Final verification: **12 biome tests passed**, the explicit benchmark passed,
**4 numeric regression tests passed**, and the captured-terrain/biome integration
test passed. The last two suites completed in 0.39 s and 49.66 s respectively,
excluding compilation. Native recapture matched with probe SHA-256
`70a44ca2058a3fa703f0c292d91d3878ac92d4e0d64ce86722e213ef9fa8202b`.
