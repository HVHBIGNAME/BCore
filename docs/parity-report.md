# BCore / Vanilla 26.1 parity report

- Seed: `846692123413862008`
- Servers: BCore `127.0.0.1:25565` (current running build); vanilla `127.0.0.1:25571`
- Samples: `[(0, 0), (1000, 0), (-2000, 3000)]`; region: `16x16`
- Status: numbers below are confirmed by this real run. BCore was not rebuilt by this harness; it measures the currently running/current release artifacts.

## Terrain height parity

| sample | columns | exact height | match |
|---|---:|---:|---:|
| (0,0) | 256 | 144 | 56.25% |
| (1000,0) | 256 | 40 | 15.62% |
| (-2000,3000) | 256 | 79 | 30.86% |

## Vegetation / surface parity

Top height is the non-air surface and is separated from terrain height; top-block compares block names at that surface. Tree-origin is the exact intersection of lowest sampled log positions.

| sample | top height | top block | vanilla origins | BCore origins | origin match |
|---|---:|---:|---:|---:|---:|
| (0,0) | 12.50% | 44.14% | 10 | 7 | 0.00% |
| (1000,0) | 8.59% | 41.02% | 16 | 10 | 0.00% |
| (-2000,3000) | 9.77% | 30.47% | 13 | 3 | 0.00% |

## Block-level diff (Y=40..120)

| sample | compared cells | equal | different | diff % |
|---|---:|---:|---:|---:|
| (0,0) | 46336 | 44662 | 1674 | 3.61% |
| (1000,0) | 46336 | 44894 | 1442 | 3.11% |
| (-2000,3000) | 46336 | 43791 | 2545 | 5.49% |
