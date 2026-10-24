# Native 26.1 loot reference

The oracle starts the pinned vanilla server, enters at `MinecraftServer.setInitialSpawn`,
uses its real `ServerLevel` and loaded datapack registries, requests one native FULL
chunk, and places a real chest. It captures `LootTable.fill` and the inherited
`RandomizableContainer.unpackLootTable` with full item components, saved NBT,
generation/split/shuffle boundaries, slot writes, and exact RNG continuation.

Read [ALGORITHMS.md](ALGORITHMS.md) for the implementation contract. The delivery
index is `target/loot-native-20261016-HANDOFF.md`.

## Pinned environment

* `target/vanilla-775/versions/26.1/server-26.1.jar`, SHA-256
  `a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`.
* `target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe`.
* `javac` 21 on PATH; reflection permits compilation without Java 25 engine types.
* Existing ASM 9.8 at
  `target/native-environment-restoration-20261014-01/asm/asm-9.8.jar`, SHA-256
  `876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051`.
* Read-only reuse of `scripts/native-generation-reference/NativeAccess.java`.

Every output directory must be new and named `target/loot-native-20261016-*`.
Sources are copied before compilation; compiled classes, source files, runtime,
libraries, input bytes, instrumentation, JAR, and native resources are hashed.
Prior inputs/captures must not be modified to make a comparison pass.

## Run

All heavy operations use the root queue; it supplies `CARGO_TARGET_DIR=target/build`,
`CARGO_BUILD_JOBS=1` and `RAYON_NUM_THREADS=2`. The capture rejects an invocation
without that queue environment. Do not nest queue runners. Wait for each predecessor's
durable status JSON before starting a command that consumes its output.

```powershell
python scripts/run_generation_check.py --background loot-native-20261016-new-first -- python scripts/loot-reference/capture.py --output target/loot-native-20261016-new-first --cases scripts/loot-reference/cases-initial.json
python scripts/run_generation_check.py --background loot-native-20261016-new-repeat -- python scripts/loot-reference/capture.py --output target/loot-native-20261016-new-repeat --cases scripts/loot-reference/cases-initial.json --repeat-of target/loot-native-20261016-new-first
python scripts/run_generation_check.py --background loot-native-20261016-new-audit -- python scripts/loot-reference/audit.py --capture target/loot-native-20261016-new-first --repeat target/loot-native-20261016-new-repeat --output target/loot-native-20261016-new-audit --plan
```

The initial input has 12 cases. `audit.py --plan` creates a bounded positive/negative
seed matrix for all available chest/trial candidate tables except explicitly
listed world-dependent exploration-map tables. `plan_components.py` chooses a few
additional inputs for the trial bottle/enchantment branches from a bounded seed
range, then the server itself must confirm the predicted function executes.
No synthetic loot tables are substituted.

The capture enforces nonzero signed-i64 seeds, null player, luck zero, at most 256
cases, containers of 0..54 slots, and a 500,000-event trace limit. Seeds are JSON
strings to preserve all 64 bits. Only size-27 cases additionally exercise the real
chest. A `preexisting` entry with `stack: null` clears that slot after optional
`fill_existing`. `missing_table_policy: "native_lookup"` explicitly exercises the
engine's missing-key-to-`LootTable.EMPTY` fallback; without that policy, missing
keys are reported rather than silently treated as an empty table.

## Outputs

* `fixtures.json`: checked native observations, including full effective component
  maps and native NBT bytes. Empty slots are null.
* `trace.jsonl`: original ordered method/RNG observations. The only repeat
  normalization removes the JVM's hidden-lambda `/0x...` suffix from `class`
  metadata. RNG values, items, slots, function results, NBT, and ordering remain exact.
* `items.json`: actual numeric item IDs, defaults, maximum stack sizes and native
  item tag membership/order. `enchantment.json` and `instrument.json` preserve
  native lookup/tag order; that order is not advertised as a numeric ID mapping.
* `potions.json`, `registry-types.json`: native potion effects and relevant registry
  IDs, including component type IDs and persistence flags.
* `loaded-tables.json`: keys of all loaded loot tables and native codec exports of
  every chest/trial candidate. `jar-resources/` preserves original JAR JSON bytes;
  `table-inventory.json` records hashes, dependencies and function candidates.
* `provenance.json`, `instrumentation.json`, `native-classes.json`, `manifest.json`:
  complete input/output and original/transformed-class identities.

`verify.py` checks preservation of occupied slots, item/component conservation
through randomized splitting, slot-pop order, direct/unpack equality, retained
seed and cleared key, no-op repeat unpack, and every recorded LCG operation and
continuation. `audit.py` also rehashes both immutable capture manifests and checks
equal sources, compilers, compiled classes, runtime libraries and contexts.

`package.py --pair FIRST REPEAT [--pair FIRST REPEAT ...] --bytecode DIR --output NEW`
verifies independent pairs before producing `loot-fill-26.1.json`, `tables.json`,
native registry inputs, a coverage inventory, and a CRC-checked evidence ZIP. The
portable fixture has plain JSON slot/item/component arrays and decimal i64/IEEE
bit strings, so a Rust test consumer needs no JVM or compiler.

## Authority limits

The native captures establish the listed finite cases. They do not establish
seed-zero sequences, player/luck branches, exploration-map state, non-chest
container invocation lifecycles, arbitrary custom datapacks, or a performance
advantage over Java. Instrumentation is for correctness research and adds overhead.
Server startup, the native chunk, and all loot bodies run inside the serialized
queue; no production BCore code is changed by these scripts.
