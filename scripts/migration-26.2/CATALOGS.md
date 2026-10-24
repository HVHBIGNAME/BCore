# 26.2 production catalog capture driver

`catalogs.py` orchestrates G1/G5–G15 captures using the existing Java probes and
metadata-only migration wrappers. Main owns execution through the one heavy-work
queue. This driver does not promote data into the crates or edit old probes.

## Commands for main

Run from the repository root. Every `--output` must be absent, with an existing
parent under `target/`. A failed run retains its files; use a fresh output for a
retry. One invocation runs its modes sequentially, without queueing child jobs.

```powershell
python -B scripts/migration-26.2/catalogs.py list
python -B scripts/run_generation_check.py --background migration-26_2-catalogs -- python -B scripts/migration-26.2/catalogs.py all --keep-going --output target/migration-26_2-catalogs-01
```

For a bounded first pass over the lightweight metadata modes:

```powershell
python -B scripts/run_generation_check.py --background migration-26_2-catalogs-blocks -- python -B scripts/migration-26.2/catalogs.py heightmap standing_blocks monster_room mineshaft_blocks mineshaft_start surface_blocks surface_climates falling_shapes --keep-going --output target/migration-26_2-catalogs-blocks-01
```

Individual mode invocation, substituting a mode from the table:

```powershell
python -B scripts/run_generation_check.py --background migration-26_2-catalog-MODE -- python -B scripts/migration-26.2/catalogs.py MODE --output target/migration-26_2-catalog-MODE-01
```

| Mode | Output inside `<output>/<mode>/` | Native source/selection |
| --- | --- | --- |
| `heightmap` | `heightmaps_26_2.json` | Original HeightmapReference, including its bounded support samples |
| `standing_blocks` | `standing_blocks_26_2.json` | Original VegetationReference.bootstrap/blockPredicates |
| `monster_room` | `monster_rooms_26_2.json` | Original MonsterRoomReference, predicates plus original bounded samples |
| `mineshaft_blocks` | `mineshaft_blocks_26_2.json` | Original MineshaftBlockReference.predicates/materials; actual loaded tags |
| `mineshaft_start` | `mineshaft_starts_26_2.json` | Loaded mineshaft biome sets/native structure iteration; dynamic decoration enum |
| `base_catalog` | `base_feature_catalog_26_2.json` | Original BaseFeatureReference `catalog <jar>` |
| `misc_states` | `misc_feature_states_26_2.json` | Original MiscFeatureReference `metadata <jar> priority` |
| `cave_blocks` | `cave_feature_data_26_2.json` | Original CaveFeatureReference `blocks` |
| `sculk_states` | `sculk_states_26_2.json` | Original SculkReference `--data-only`, retaining only its `data` catalog |
| `surface_blocks` | `surface_builder_blocks_26_2.json` | Original SurfaceBuilderReference.bootstrap/blockPredicates |
| `surface_climates` | `surface_climates_26_2.json` | Loaded biome base-temperature float bits and native temperature modifier |
| `extra_tree` | `extra_tree_catalog_26_2.json` | Original ExtraTreeReference with `BCORE_EXTRA_TREE_PROBE` removed |
| `falling_shapes` | `falling_block_shapes_26_2.json` | Original FallingShapeReference; virtual delays and original edge callbacks |
| `lighting_catalog` | `lighting_catalog_26_2.json` | Original LightingReference `catalog <jar>` |
| `lighting_updates` | `lighting_updates_26_2.json` | Original LightingReference `updates <jar>` |
| `scattered_catalog` | `scattered_structure_catalog_26_2.json` | Original catalog/blockProperties, without admission/layout suites |
| `pyramid_catalog` | `desert_pyramid_catalog_26_2.json` | Original DesertPyramidReference.pyramidCatalog |
| `scattered_containers` | `scattered_containers_26_2.json` | Original defaults/seeded deferred container saves; typed NBT projection |
| `brushable` | `brushable_block_entities_26_2.json` | Original BrushableRuntimeReference; item names and load/save cases |
| `jigsaw_assets` | `pool_alias_trial_combined_assets_26_2.json` | Original PoolAliasTrialAssetsBootstrap/JigsawAssets and native template manager |
| `fossil_assets` | `fossil_assets_26_2.json` | Original FossilAssets; native manager/data fixer and resource digests |
| `trig_table` | `misc_feature_trig_26_2.json` | Original make_table function; independently derived mathematics, not native evidence |
| `spawn_assets` | `generation_spawn_assets_26_2.json` | Metadata-only actual-server boundary: catalog, predicates, native constructors |
| `spawn_callbacks` | `generation_spawn_callbacks_26_2.json` | Actual-server constructor/sensor metadata and native memory intervals |
| `spawn_handoff` | `generation_mob_catalog_26_2.json` | Original GenerationSpawnHandoff.catalog; native metadata/default codecs |

`jigsaw_assets` defaults to ancient_city, village, trail_ruins, trial_chambers and
pillager_outpost. `--family` is repeatable for a narrower independent capture.
`--family spring` selects the sulfur-spring templates explicitly and keeps the
five-family default unchanged.
Native template loading, data fixing, rotation/mirror tables and original NBT
bytes stay linked; a raw template copy is not substituted for native decoding.

Spawn modes require an already installed ASM JAR. The default is the independently
pinned ASM 9.8 file used by the historical capture track:

```text
target/native-environment-restoration-20261014-01/asm/asm-9.8.jar
876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051
```

Main can supply `--asm <path> --asm-sha256 <digest>` for another authenticated
local tool artifact. The driver never downloads tools. Spawn modes stop at the
original MinecraftServer.setInitialSpawn entry, prohibit gameplay ticks, and
reuse the original explicit entity-entropy helper. They do not claim generation
task history, finalizer callback sample parity, or packet delivery coverage.

## Provenance and bounds

- `native.environment()` authenticates the official target JAR and every prepared
  library, including exact directory membership. This driver requires 39 libraries.
- The fixed target is `target/vanilla-776/versions/26.2/server-26.2.jar`, SHA-256
  `183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8`.
- `environment.json` binds preparation, JAR, library, Java/javac/Python executable
  digests, version logs and relevant runtime/queue environment. Modes reference it
  by digest.
- Each mode freezes exact Java sources and Python projection helpers under
  `sources/`, then compiles only those snapshots. `capture-inputs.json` records
  source order, per-file hashes, marker, configuration and projection identity.
  Sources present in the preparation baseline must still match their historical
  hashes. Spawn runs compile and execute against an exclusive frozen ASM copy.
- Each subprocess has exclusive stdout/stderr logs, command JSON and result JSON.
  Working and Java/OS temporary directories are inside the fresh target output.
  Default retention is 64 MiB per stream; excess output fails the capture and
  terminates the owned subprocess. Compilation is bounded to 120 seconds; native
  captures use 600–1200 seconds. Override with `--max-log-mib` or `--timeout`.
- `native-result.json` retains the unmodified parsed probe payload. Final catalogs
  carry target identity and exact concatenated Java-source hash; the bare climate
  map keeps its runtime schema and uses the manifest for provenance.
- `manifest.json` authenticates the catalog, snapshots, compiled classes, commands,
  logs, agent and projected resources. Generated server world files are explicitly
  outside this catalog manifest. `results.json` reports per-mode pass/fail status.
- State catalogs are checked against 32366 states; climate/biome catalogs against
  66 entries. Registry ranges and jigsaw IDs must cover the complete native order.
  Native serialization failures and conflicting version metadata fail closed.

## Compatibility evidence

The catalog track has captured all 25 modes, including the repaired surface
wrapper and the actual-server spawn boundary. Retained first-run failures identify
the target API changes. Runtime template, speleothem, tree and loot captures use
separate repeat-verified fixtures.

- CaveFeatureReference still selects the historical `pointed_dripstone` and
  `dripstone_cluster` documents/tags; the target replaces their feature classes.
  Its original `blocks` mode is intentionally tried first.
- SculkReference classifies dynamic-shape owners by old class name, including
  PointedDripstoneBlock. Inspect `unsupported_shape_blocks` before promotion.
- Base/extra-tree probes exercise every registered plant's native survival
  methods; new contextual plants may reject their explicit fixture world.
- Structure probes depend on current template-manager internals and item-component
  initialization. Original probe failures must not be converted to guessed data.
- Spawn wrappers validate the actual setInitialSpawn descriptor and the native
  Entity constructor RandomSource.create instrumentation site. API changes in
  spawning/variant/metadata methods must be fixed in migration-owned copies.
- ASM 9.8 must admit the target class version. If it does not, main must supply a
  verified compatible ASM tool artifact before retrying.

Promotion remains main's responsibility. Historical outputs, old scripts and
production Rust are not destinations for this driver.

## Offline validation

```powershell
python -B scripts/migration-26.2/test_catalogs.py
```

These eleven tests exercise CLI exclusion before bootstrap, explicit spring-family
selection, native marker/identity
admission, ordered-state range completeness, historical source pins, typed NBT
widths/empty-list element type, output-overflow termination and subprocess
deadlines. Native environment and process entry points are patched; process
fixtures are in-memory only and the tests write no capture files.

## Registry source reproducibility

`registry_data.py` rebuilds the registry constants, climate rows and worldgen
documents from the authenticated core capture and datagen reports. The original
publication remains recorded in `crates/bcore-registry/data/provenance_26_2.json`.
`generated_source_26_2.json` binds the formatted Rust source to that publication,
the current generator and the unchanged registry data digests.

```powershell
python -B scripts/run_generation_check.py --background migration-26_2-registry-verify -- python -B scripts/migration-26.2/registry_data.py --output target/migration-26_2-registry-verify-NEW --verify-published
```

The verifier checks the complete generated source with explicitly recorded UTF-8
CRLF-to-LF normalization, and the native registry, climate and worldgen data
byte-for-byte. `published_verification.json` retains raw checkout digests and the
generated digests for every compared file. The preserved original source, native
capture and datagen hashes are also checked. Its output directory must be new.

`catalog_projection.py` resolves strict JSON Pointers and reports the complete
source-byte digest. `block_entity_replay_audit.py` requires both full state
inventories and exact captured JAR identities, joins complete state properties and
retains the original template row coordinates. Their offline regression suite is
`test_reference_inspection.py`.
