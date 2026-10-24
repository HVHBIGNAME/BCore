# Performance experiments — 2026-10-20

## Accepted: selective outlining of density branches

**Both alternating confirmation runs improved at all worker budgets, and all
18 individual before/after process-median pairs improved.** The production change
is confined to `crates/bcore-worldgen/src/density.rs`. It keeps exact arithmetic,
cache keys, node identities, phase order and shared-corner reuse.

| Workers | Paired gain, run 1 | Paired gain, run 2 | Same-run Java chunks/s | BCore chunks/s |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1.242989× | 1.359483× | 42.2792 | 24.2225 |
| 2 | 1.309529× | 1.253988× | 92.2836 | 41.2502 |
| 4 | 1.394446× | 1.293477× | 175.2779 | 85.2307 |

The measured gain is **24–39% over the preceding BCore**, depending on run and
worker budget. BCore remains slower than Java in this NOISE workload. This is not
a whole-server or complete-generation throughput comparison.

### CPU attribution and mechanism

A user-mode sampler inspected only its owned diagnostic benchmark child. It used
Windows `SuspendThread` / `GetThreadContext` / `ResumeThread`, an activity filter,
and local PDB line tables; it changed no host profiling policy. Baseline and
candidate builds used release optimization plus the explicitly recorded
`CARGO_PROFILE_RELEASE_DEBUG=1` diagnostic override.

The baseline produced 8,488 leaf samples, 5,565 attributed to the density evaluator
including inlined work. These are statistical observations, not exact CPU
percentages. The full benchmark process also includes warmups and fingerprinting;
sampled elapsed/batch times are not used as throughput results.

Disassembly established the actionable code shape:

| Density evaluator entry | Baseline | Outlined diagnostic build |
| --- | --- | --- |
| Reserved frame | `0x2c8` bytes | `0x158` bytes |
| Unconditional nonvolatile SIMD saves | XMM6–15 | XMM6–11 |
| Before selecting the node | X/Z `floor` calls, casts and quart/bounds preparation | Dispatch without coordinate rounding |

The optimized evaluator delegates `flat_cache`, `interpolate`, seeded blended
noise lookup and Blender-coordinate helpers through explicit non-inlined
boundaries. The arithmetic bodies are unchanged. This confines register-heavy
interpolation and coordinate preparation to the selected branch instead of
imposing them on every recursive constant/arithmetic/cache node.

Local profile evidence:

- `target/noise-cpu-profile-20261020-01/`: baseline; profile SHA-256
  `8e1e875ef4af080b59b5e9ba2b0b49700a69c25cb9b35e4e2dfcd4f4b3bf66bc`.
- `target/noise-cpu-profile-20261020-02/`: candidate, 5,992 leaf samples; profile
  SHA-256 `3542fcafdd12910ebf2facc61a6c622663e92a5214fb0a9ecceaaa052452183b`.
- The independent source/compiled-shape review is retained in
  `target/noise-outline-20261020-review.md`. Profile binaries are separate from
  the default-release frozen throughput binaries.

### Correctness and full checkpoint

- **782 tests passed, zero failed, four ignored, 66 test/doc-test targets** in a
  fresh full-workspace release run. Release build, formatting, diagnostics examples
  and server `--help` passed. Existing dead-code/deprecated diagnostic warnings
  remain; this change introduced no new warning.
- All **54 process records** across the full Java/BCore run and two paired runs
  matched native block, ordered fluid-mark and both WG-heightmap fingerprints.
- Four fresh schema-3 histories use one newly built frozen replay executable:
  **27/27 requested** and **52/54 watch** snapshots match scored fields. Their
  complete request-comparison facets equal the preceding chest checkpoint.
  The same four LIGHT/SPAWN fields in two watches remain different; coverage
  remains Partial.
- All 417 MAIN source inputs matched the measured snapshot throughout the final
  checks. The earlier admitted baseline's ten EOL-only differences are recorded
  below. No native expected data, workload or harness contract was changed.

| Evidence | SHA-256 |
| --- | --- |
| `target/noise-fill-wg-v2-outline-20261020-01/results.json` | `18e18a5547d8988d9c5ac651b33fd09dc63a8ab1b79f99db76d582ad6b978ad2` |
| `target/noise-fill-wg-v2-outline-paired-20261020-01/results.json` | `af0fe01ce5666e9c477f1d9b575808fee47ea2b91949703c4f00f8d4feeeb684` |
| `target/noise-fill-wg-v2-outline-paired-20261020-02/results.json` | `b4f6382290ce970af1192ffbe973afed5e0bc2a75d0baa916fc1d8bc9ce0ab62` |
| `target/noise-outline-checkpoint-20261020-01/results.json` | `328b5c8d2603abbcbd5175a2d74c0907ba6edc4d205c20a3cd069b6e7179139c` |
| Accepted `density.rs` | `3dc532828098efe63fcb53b091bed4e470c70d5a60d555c3312ace3351712bc6` |
| Frozen server executable | `a4d735bd44c2a72918d113e254c14c98f7dd7add1d44991db69a6faab7fb903e` |
| Frozen schema-3 replay executable | `21e1eee166fd5362854086d8e5e3ab21098612000c29986a4867ee2d1d2517f5` |

Published evidence preserves all process medians, measured/warmup batches and
artifact identities:

- [Performance](metrics/noise-fill-wg-v2-outline-2026-10-20.json).
- [Workspace and replay validation](metrics/noise-outline-validation-2026-10-20.json).

Heavy-work jobs:
`noise-outline-measure-20261020-01-1792531733055218000`,
`noise-outline-checkpoint-20261020-01-1792535007586864000`,
`noise-outline-export-20261020-01-1792535476714232200`, and
`noise-outline-replay-summary-20261020-01-1792538834673007900`.
All finished with exit 0. Source freeze for this completed checkpoint is released.

## Coordinate-hash rotation: rejected

The candidate rotated `FastHasher::finish()` left by 16 bits. Full generation did
not show a repeatable improvement, so the production change and its new test were
removed. The original `fast_hash.rs` was restored; accepted shared-corner reuse and
the loot/FULL/request changes were retained.

### Motivation and isolated measurements

Integer-valued `f64` coordinates carry much of their variation in their high bits.
The existing final multiply leaves poor low-bit distribution for these keys. An
isolated Rust diagnostic used the production hasher and the same six-field key
shape, with one fixed node, seed and regular coordinate lattices.

For the 98,304 origin block keys and 131,072 projected home buckets:

| Hasher | Occupied home buckets | Maximum keys at one home | Median ns/hit |
| --- | ---: | ---: | ---: |
| Original | 384 | 528 | 237.62 |
| Original plus 64-bit finalizer | 69,121 | 7 | 115.24 |
| Original plus final rotation | 70,730 | 5 | 108.79 |

These are synthetic map measurements, not a CPU profile or generation throughput.
Home-bucket occupancy does not measure the actual probe-chain length. Rotation is
bijective and retains the original full-hash collisions: 98,236 distinct hashes
for these 98,304 keys. Complete key comparison still distinguishes all entries.

The 90-observation diagnostic, all repeats and its source/binary identities are in
`target/noise-hash-profile-20261019-02/results.json`, SHA-256
`bb1b0fcbed934f2e823bd258507d15c6a3e961fdf1b21d91f33fa9ec7f77d7bc`.
The preceding finalizer-only experiment is retained in
`target/noise-hash-profile-20261019-01/`.

### Production gate

The unchanged `noise-fill-wg-v2` contract used seed `846692123413862008`, the same
eight chunks, 1/2/4 workers, three fresh processes per configuration, ten warmup
batches and ten measured batches. Each throughput below is eight chunks divided
by the median of the three process-median batch times. No samples were discarded.

| Workers | Same-run Java chunks/s | Candidate BCore chunks/s | Paired gain, run 1 | Paired gain, run 2 |
| --- | ---: | ---: | ---: | ---: |
| 1 | 58.6388 | 20.3219 | 0.968023× | 0.971590× |
| 2 | 97.1467 | 33.8513 | 1.074497× | 0.976887× |
| 4 | 166.3679 | 58.8387 | 1.077166× | 0.952823× |

The BCore-only paired runs alternated the frozen baseline and candidate binaries.
The one-worker regression repeated; the apparent multi-worker improvements did
not. The candidate did not outperform Java at any worker budget.

All **54 process records** across the full run and both paired runs matched the
native block, ordered-fluid-mark and both WG-heightmap fingerprints. These runs
exclude later generation stages, chest opening and gameplay.

| Result | SHA-256 |
| --- | --- |
| `target/noise-fill-wg-v2-chest-checkpoint-20261018/results.json` — baseline | `39fa9f594b21076349228de1959ef7a22cabd52fe524599c94d40dc3e820105e` |
| `target/noise-fill-wg-v2-hash-20261019-01/results.json` | `8a816b58b589bf18be1a4eb96772bcddfff608c42255d83e579e0618398a3ee6` |
| `target/noise-fill-wg-v2-hash-paired-20261019-01/results.json` | `e105c60fa6020b93cb41f7d7a2ec7289bb16187cff0677aef1fb7b5396456292` |
| `target/noise-fill-wg-v2-hash-paired-20261019-02/results.json` | `fc7ac95382b94962d1bc7a0b05e163071369c6d69ffab58aa56a5d037f71d7ee` |

The candidate executable SHA-256 is
`6c256f6a633982f6276b63fbe3806a46058b96bbd9e4d070a699f89b7dc5a658`.
Frozen sources, binary, exact requests, all timings and build/environment
provenance remain beside each result.

### Correctness and preservation

- Candidate release checks: **340 library tests passed, two ignored**, and all
  four native numeric-reference tests passed (895 native cases / 62,649 values).
  Formatting passed after one new-test-only formatting correction.
- This is candidate-specific coverage, not a new complete-workspace checkpoint.
- The admitted working tree contained 417 benchmark source inputs. Ten files
  differed from the Oct-18 snapshot only by CRLF/LF; the diagnostic verified the
  original hashes and byte equality after only line-ending normalization.
- During the production experiment only `fast_hash.rs` differed from that admitted
  tree. The gate verified that all MAIN inputs remained unchanged during all three
  measurement stages.
- `target/noise-hash-gate-20261019-01/` retains the candidate source, prerequisite
  receipts and source identities. The complete gate job was
  `noise-hash-measure-20261019-01-1792497031559955100` (exit 0 means matching outputs
  and completed measurements, not accepted performance).

The synthetic hash-table improvement alone did not identify a useful production
optimization; the subsequent CPU profile led to the accepted outline above.
Source-backed alternatives are catalogued in
[the native competitor research](native-performance-research-2026-10-19.md).
