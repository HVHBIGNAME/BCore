# 0.7.0-alpha.1 — worldgen foundations and honest parity

Development remains on protocol **775**. This is an **alpha**, not a complete
vanilla-compatible server. Java plugin API expansion is deferred.

## Changes
- Embedded worldgen data: standalone binaries and tests use the same generator.
- Structural climate parsing, quantized distance and complete 3D biome palettes.
- Leaf-litter placement, corrected selector defaults and chunk-edge wrapping fix.
- NormalNoise sampling without per-sample permutation-array cloning; bounded
  worker density caches.
- Bounded chunk-generation lookahead and less competition between whole-chunk
  jobs. Spawn height uses generated/saved terrain instead of prototype noise.
- Client permission events on join, `/op` and `/deop`; operator authorization for
  the F3+F4 switch packet.
- Coordinate-addressed parity captures, offline vanilla references and refreshed
  GitHub Pages statistics. The old tracker produced NaN for `in progress` entries.

## Measured scope
One seed, three 16×16 regions: **768/768 terrain heights**, **4608/4608 biome
cells**, **95.85% exact block states including air**. The old low terrain figures
were a transposed-grid measurement bug. Trees, ores, aquifers, structures and
cross-chunk decoration remain incomplete; one sampled region regressed in block
match while the other two improved. See `docs/parity-report.md` for raw counts.

Existing saved chunks keep their previous generator output. For visual comparison,
run the binary in a separate empty working directory instead of deleting an
existing world. The executable contains its worldgen data; no JAR is needed to run.
