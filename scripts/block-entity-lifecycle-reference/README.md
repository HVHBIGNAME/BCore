# Native deferred block-entity lifecycle reference

The probe targets Minecraft **26.1 / protocol 775**, server JAR SHA-256
`a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`.
It records the distinction between proto pending NBT, live block entities,
save/load boundaries, and client chunk update tags.

`LifecycleAgent.java` enters at `MinecraftServer.setInitialSpawn` on a real
`ServerLevel`, before gameplay ticks. `LifecycleProbe.java` constructs small air
ProtoChunks and holders, then invokes original Minecraft methods through
`NativeAccess`. Its scope is individual lifecycle transitions; actual ChunkMap
ticket scheduling remains covered by the native generation-history captures.

## Evidence

The v1 fixture contains **39 cases** and **55 native save/read round trips**:

* 16 ordinary writes: eight block types, each with flags 2 and 18.
* Four explicit region callbacks/lookups, including loot tables and hive bees.
* 15 template placements: absent NBT, empty NBT, and nonempty payloads.
* Four saved-pending loads.

Observations include direct proto reads, region lookups, native chunk save/read,
LevelChunk construction/saving, and `BlockEntityInfo.create/write` packet bytes.
Two independently bootstrapped server processes produced identical observations
with zero gameplay ticks.

* Fixture: `crates/bcore-worldgen/data/deferred_block_entities_26_1_v1.json`
* Evidence: `fixtures/deferred-block-entities-26_1-v1.zip`
* Observation SHA-256:
  `3f42bcdf85469b8bae577c715d1c3a6240b855892c0367ff8843dcecad63a2ca`
* Fixture SHA-256:
  `25918191969edfb8483f227cf35b70ee924f3ec3d52ede8078ccd18c7fdad006`
* Archive SHA-256:
  `ea26ff3668ea1988a2fa2503c2500b7e419b44963428e9e3392961da20b5c38e`

The archive retains each run's raw NBT/observations, decoded observations,
manifests, native bytecode listings, logs, probe sources, and provenance.

## Reproduce on the Windows worker

Run from the detached worker root. Prerequisites are the coordinator's read-only
`target/vanilla-775` and `target/jre25` caches, compatible `javac`/`javap` on PATH,
and network access for the pinned ASM 9.8 dependency. Every output directory must
be new. Check each background job's durable status before starting a dependent
command.

```powershell
python ..\run_check.py --background block-entities lifecycle-capture -- python scripts/block-entity-lifecycle-reference/capture.py --output target/be-lifecycle-new-first
python ..\run_check.py --background block-entities lifecycle-repeat -- python scripts/block-entity-lifecycle-reference/capture.py --output target/be-lifecycle-new-repeat --repeat-of target/be-lifecycle-new-first
python ..\run_check.py --background block-entities lifecycle-pack -- python scripts/block-entity-lifecycle-reference/pack_fixture.py target/be-lifecycle-new-first target/be-lifecycle-new-repeat --fixture target/be-lifecycle-new-fixture.json --archive target/be-lifecycle-new-evidence.zip
```

Packing verifies both manifests, identical decoded observations, and matching
native JAR/source/runner/settings provenance. Existing fixture/archive paths are
rejected. A recapture's archive hash can differ because logs and provenance
contain run-specific paths; the decoded observations are compared exactly.

## Rust checks

```powershell
python ..\run_check.py --background block-entities lifecycle-worldgen-tests -- cargo test --release --locked -j 1 -p bcore-worldgen --lib deferred_ -- --test-threads=2
python ..\run_check.py --background block-entities lifecycle-protocol-tests -- cargo test --release --locked -j 1 -p bcore-protocol --test deferred_block_entities -- --test-threads=2
```

The tests exercise source-owned lazy lookup, typed pending data, hive and loot
callbacks, atomic materialization errors, the BE-only FULL-request boundary,
BCC v5 with legacy readers, and native packet projection. FULL completion and
implicit ticket-driven promotion remain explicitly pending in generation
coverage. The final repeated-LIGHT history still exposes that missing transition.
