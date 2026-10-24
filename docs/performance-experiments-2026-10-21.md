# NOISE optimization — 2026-10-21

## Final retained implementation

Selective outlining, chunk-owned indexed interpolation and borrowed-point scalar
evaluation are retained. Two **direct frozen comparisons against the October 18
chest checkpoint** measured the complete series together:

| Workers | Cumulative gain, run 1 | Cumulative gain, run 2 | Final Java chunks/s | Final BCore chunks/s |
| --- | ---: | ---: | ---: | ---: |
| 1 | 2.075546× | 2.027788× | 44.1602 | 39.5998 |
| 2 | 2.453408× | 2.249699× | 88.4344 | 71.1340 |
| 4 | 2.138244× | 2.017397× | 198.4149 | 139.9994 |

All 18 individual cumulative pairs improved. The full Java/BCore run and both
cumulative comparisons retain **54 matching process records**, with unchanged
blocks, ordered fluid marks and both WG maps. These directly measured gains
replace any attempted multiplication of stage gains from different sessions.
Java remains faster at all three budgets in this final run; exceeding Java on
all budgets has not been established.

The final source snapshot passed **791 workspace release tests**, zero failures,
four existing ignored tests and 66 test/doc-test targets. Format, workspace
release build, diagnostics examples and the frozen server's `--help` passed.
All **419 source inputs** remained identical through validation and measurement.
The native numeric suite retains its 895 cases / 62,649 values, including
reverse-order/four-worker checks.

Four histories used one freshly built schema-3 replay executable. They retained
**27/27 requested** and **52/54 watch** scored matches. Every request-comparison
facet is identical to the preceding outline checkpoint. The same four
`persisted_status` / `serialized_status` fields differ at request index 2's origin
watch in N1 and N2: native `minecraft:light`, BCore `minecraft:spawn`. All requests
still report Partial implementation coverage. These four histories do not resolve
the separate wider 91-request corpus or establish all-seed/all-dimension parity.

Published evidence:

- [Final Java/BCore run](metrics/noise-fill-wg-v2-final-2026-10-21.json).
- [All stage, rejected-candidate and direct cumulative measurements](metrics/noise-fill-wg-v2-series-2026-10-21.json).
  This records 378 process runs across seven experiments, with all warmup/measured
  durations and process medians. They revisit the same bounded world positions.
- [Fresh workspace/build/replay checkpoint](metrics/noise-final-validation-2026-10-21.json).

## Chunk-owned indexed interpolation: performance gate passed

Two frozen alternating comparisons improved at 1/2/4 workers, and all 18 paired
process medians improved. The comparison is against the accepted
[selectively outlined evaluator](performance-experiments-2026-10-20.md), not the
older unoptimized dispatcher.

| Workers | Paired gain, run 1 | Paired gain, run 2 | Same-run Java chunks/s | Indexed BCore chunks/s |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1.557084× | 1.238053× | 36.8025 | 25.7407 |
| 2 | 1.392881× | 1.625257× | 58.8476 | 55.2528 |
| 4 | 1.343235× | 1.625803× | 135.6215 | 103.5323 |

BCore remains slower than Java in this full-run measurement. Historical timing
ratios and products of gains from different sessions are not a cumulative
performance measurement.

### Implementation

`density/chunk.rs` borrows the actual parsed graph for one full NOISE job and
compiles its small scalar skeleton above interpolation boundaries. The bundled
roots contain five final-density cuts, one vein-toggle cut and two vein-ridge
cuts; vein-gap remains generic. A valid cell hit indexes the node's own array and
uses the shared phase-specific arithmetic helper. It avoids the former
six-field coordinate-map lookup rather than adding a front cache before it.

- The full job's fixed integer geometry gives 768 cells per interpolator. Eight
  nodes reserve 384 KiB of corner payload plus 768 validity bytes before metadata.
  Storage is temporary and drops at job completion/unwind.
- Cell misses retain the original `cache_once` vertex sharing and exact eight
  corner request order. A cell is valid only after every corner succeeds.
- Final-density interpolation uses X/Y/Z; live material uses Y/X/Z. Nonlinear
  Squeeze/Min/Range operations stay outside their original interpolation cuts.
- Unsupported marker subtrees and off-job typed points use the original generic
  evaluator. Raw/fractional/custom-Blender entry points retain that path.
- Graph/context lifetime belongs to the job. No indexed cache survives into
  Proto/FULL holders, block entities or persistence.
- The original material loop, Beardifier addition, biome-query history, aquifer
  barriers, lazy ridge/gap demand and random draw order are retained.

Admission caps the entire job at 16 interpolators, each root at 4,096 scalar nodes,
and traversal at 128 levels. The compiler reserves nodes before descending and
tracks memoized subtree heights; an unsupported size declines the optimization
without publishing a partial plan. Review found this guard issue in the first
candidate and it was corrected before performance measurement.

### Indexed-stage validation

The indexed candidate passed **348 release library tests**, two ignored, and all
four native numeric-reference tests. Nine new behavioral tests include:

- All four root bits at all 98,304 block points in each of two seed/chunk cases,
  forward, warm reverse and freshly compiled cold reverse: 786,432 reference
  values and 2,359,296 indexed comparisons.
- Complete integer geometry, boundary/corner admission, independent seed/job
  scopes, lazy Range branches and no old interpolation-table entry on the indexed
  path.
- Failed-corner publication, bounded deep/unselected/shared graphs, mixed generic
  cache-marker material coercion, direct NaN/±0/subnormal/Min/Max/Squeeze bits and
  reader-demand order through the actual vein bridge.

The inherited native material/structure/mark tests ran with the integrated full
NOISE path. All **54 benchmark process records** across the full run and both
paired runs matched native blocks, ordered fluid marks and both WG maps.
The benchmark contract remains eight chunks, seed `846692123413862008`, three
processes per worker configuration, ten warmups and ten measured batches. All
timings and process variation are retained.

The later 791-test workspace checkpoint above validates the retained indexed and
borrowed-point implementation. The earlier 782-test checkpoint belongs to the
outlined baseline and remains a separate execution.

### Evidence

| Result | SHA-256 |
| --- | --- |
| `target/noise-fill-wg-v2-outline-20261020-01/results.json` — baseline | `18e18a5547d8988d9c5ac651b33fd09dc63a8ab1b79f99db76d582ad6b978ad2` |
| `target/noise-fill-wg-v2-indexed-20261021-01/results.json` | `39f684eaab93d0cb911575a3cdb81422755c220978ba49ccf11ecde862022bbe` |
| `target/noise-fill-wg-v2-indexed-paired-20261021-01/results.json` | `016571859c7e7dc65ab59e2cfcd8c68ac7503bb2c683ea9ff68247ed9bcc7dcd` |
| `target/noise-fill-wg-v2-indexed-paired-20261021-02/results.json` | `89eae22267854978a02f218d77d6ae39af5521a7ab8ee6540239af7db3237158` |

The ROOT gate job is `noise-indexed-measure-20261021-02-1792544191929006200`,
finished with exit 0. Frozen sources, binaries, exact requests and complete
provenance remain alongside the results. Initial new-test-only formatting
failures and the refused premature gate are retained in `target/generation-checks/`;
the successful gate required the corrected test and formatting receipts.

## Borrowed-point evaluator: retained, small measured gain

`RootDensity::evaluate` and `CellInterpolator::sample` now borrow the one
`BlockPoint` constructed by `RootDensity::sample`. Inspection of the retained
diagnostic executable confirmed removal of both 72-byte recursive point-copy
blocks. The scalar evaluator's stack reservation changed from **168 to 88 bytes**;
the selected Range branch became an in-frame loop.

| Workers | Borrow gain, run 1 | Borrow gain, run 2 |
| --- | ---: | ---: |
| 1 | 1.018088× | 1.034403× |
| 2 | 1.021965× | 1.018687× |
| 4 | 1.044942× | 1.013704× |

Aggregate medians improved in both confirmations, but **two individual pairs
regressed**. This is a small, mixed-pair result on an unisolated host, separate
from the larger indexed and final cumulative gains. Its full benchmark and both
comparisons have 54 matching process records. The 348-library / 4-numeric checks
and format check passed before measurement.

## Further candidates: rejected and restored

Each candidate was compared against the retained indexed-borrow binary, using
the same eight chunks, seed, three fresh processes, ten warmups and ten measured
batches at 1/2/4 workers. Matching fingerprints alone did not admit an optimization.

| Candidate | Run 1 gain, 1 / 2 / 4 workers | Run 2 gain, 1 / 2 / 4 workers | Decision |
| --- | ---: | ---: | --- |
| Outline cell miss | 1.030591 / 0.958117 / 1.023016× | 1.028842 / 1.029989 / 0.995911× | Mixed budgets; rejected |
| Constant-operand lowering | 1.017586 / 1.012517 / 0.983967× | 1.019063 / 1.012603 / 1.028174× | Small, mixed 4-worker result; rejected |
| Perlin cast-plus-correction | 0.952180 / 0.966094 / 0.960330× | 0.960592 / 0.961976 / 0.954825× | All aggregate medians regressed; rejected |

- **Cell miss:** a private outlined fill preserved corner order and publication
  after success, but did not sustain improvement across budgets. It was restored
  to the original combined `sample` body. A standalone full run in which this
  rejected candidate beat Java at one or two budgets is not the retained result.
- **Constant operands:** retained original operand placement and floating
  operations while avoiding recursive Constant dispatch. Both source changes and
  candidate-specific test changes were restored after the mixed result.
- **Perlin rounding:** existing PDB/leaf-IP samples and bounded AMD64 disassembly
  identified three coordinate `floor` calls in `ImprovedNoise::noise`. The
  attempted replacement was `value as i32` followed by a saturating one-step
  negative-fraction correction. It passed **350 library and four numeric tests**,
  including integer/saturation boundaries, signed zero, subnormal/NaN/infinity
  patterns, one million deterministic IEEE patterns and lattice-boundary noise
  comparisons. All 54 benchmark records matched, but throughput regressed by
  about 3–5%; its runtime and two tests were removed. The first interrupted test
  attempt (`0x40010004` / `0xC000026B`) is retained separately from the successful
  retries and is not counted as a passing run.

The restored MAIN source is byte-identical to the accepted indexed-borrow
419-input manifest. `noise_perlin.rs` and `fast_hash.rs` have no diff from HEAD.
No rejected candidate contributes to the production speedup.

## Final artifact identities

| Artifact | SHA-256 |
| --- | --- |
| `target/noise-final-checkpoint-20261021-01/results.json` | `81c2e8d4e5cf6085416e03ac97bbc9bbf865c85c19e423c364ebea5578d93eba` |
| `target/noise-fill-wg-v2-final-20261021-01/results.json` | `7f1eec909a50feb0d757e52802468cdc0e6e574ce3861a26a8836e458dba53ed` |
| `target/noise-fill-wg-v2-final-cumulative-paired-20261021-01/results.json` | `dbe3e6ad026a3ab18c36b3547dbaa070efa3b2301b6035eb530670304267d1b4` |
| `target/noise-fill-wg-v2-final-cumulative-paired-20261021-02/results.json` | `5a9436291be430a9f04665dd51830628eaf0126fd2184b9b5b0090becc345bbe` |
| Final benchmark executable | `9a9e2b6479dbca796ed0394b8160d4db34e5620da9e43f477cdf77fb0fcd1f10` |
| Frozen server executable | `a6451f50b4975b74271182389fb5e2839299c2f50207cff812c0dced92e00435` |
| Fresh native-history replay executable | `81bca5a5d3764719536cd0c32e6a42c46f5614ee6d9b2b32cce0dd717ac3ffc0` |
| Final source archive | `56fcf6f3a60b581e90be574bbb61cd42d59e3d00ca480a9fd72e08bf61b1b674` |
| Final source manifest | `aa1dc6b2b7572d32c914c22bcdcad7ecaea4baa543e7702d2fc0da1b3f306714` |

CPU profiles use a separate release build with line tables and owned-child
sampling. Sample counts and sampled elapsed times are not throughput evidence.
Inherited dead-code/deprecated diagnostic warnings remain; a complete aislop
scan was unavailable. Future work should start from this retained checkpoint and
the remaining profile hotspots, with a new isolated candidate and the same
numeric/frozen measurement controls.
