# Generated tree effects and tick preparation (26.1)

Generated beehive occupants and deferred block/fluid tick requests now transfer
from `TreeEffects` into their owning chunks and survive **`.bcc` v4** save/load.
Native-reference-tested helpers also implement chunk-local tick containers.
The server's shared generation context retains neighbouring effects across
requests. Native schedule parity, runtime tick-queue handoff and gameplay tick
execution remain pending; see [feature-scheduling.md](feature-scheduling.md).

## Owning-chunk transfer

`FeatureRegion::transfer_tree_effects` validates the staged batch before consuming
it. Hive positions must contain compatible blocks; requests must have valid
target IDs and build heights. An invalid batch retains its staged effects and
stored data for retry. A successful transfer:

- writes each complete hive snapshot to the chunk at `(x >> 4, z >> 4)`;
- appends requests to that owner's `GeneratedChunk`, preserving existing requests,
  insertion order, duplicates and signed delays;
- clears the staging buffers, so repeating an empty transfer does not duplicate
  occupants/requests or copy unchanged chunks.

Restaging a hive after transfer includes its existing occupants. Native proto-chunk
writes preserve occupants across identical states, facing/honey changes and
nest-to-hive transitions. Replacing the hive with stone or air removes its data;
recreation starts empty. Raw tick requests survive these writes.

[`tree_effects_26_1.json`](../crates/bcore-worldgen/data/tree_effects_26_1.json)
checks **18 native cross-chunk traces**, nine scenarios with flags **19** and **3**,
through actual `WorldGenRegion.setBlock` and `ProtoChunk` block-entity lookup.
The Rust replay checks staged, transferred and finalized data (**54 replays**).
Transitions to other block-entity types are outside this fixture's scope.

`GenerationWorld` transfers effects after each source attempt, including failures,
and retains neighbouring chunks for subsequent requests. Invalid staged batches
remain attached to the failed source. `ChunkColumn::from_generated` carries the
returned chunk's entities, requests and unconsumed postprocessing marks into the
protocol/storage layer. Legacy target-local helpers are retained for fixtures.

This cross-request ownership is currently in memory. The column store does not yet
persist the holder graph or hydrate its saved neighbours; those restart semantics
remain separate work.

## Beehive data and native evidence

[`beehives_26_1.json`](../crates/bcore-worldgen/data/beehives_26_1.json), captured by
[`BeehiveReference.java`](../scripts/BeehiveReference.java), contains **100 native
samples**, all **48 compatible states**, four incompatible-state checks and
**30 codec cases**. It exercises actual native construction, occupant storage,
save/load, typed NBT and update serialization. BCore checks generated metadata
values and exact client update bytes, plus its own `.bcc` round trips.

`BlockEntity::Beehive` stores ordered `ticks_in_hive: Vec<i32>`. Its generated
metadata has `id = "minecraft:beehive"`, coordinates, empty `components`, and a
`bees` list whose entries contain:

- the signed `ticks_in_hive` value;
- `entity_data = {"id": "minecraft:bee"}`;
- `min_ticks_in_hive = 600` for generated nectarless occupants.

All facing/honey states of bee nests (**21768–21791**) and beehives
(**21792–21815**) accept this data in the pinned registry. Native codecs accept
signed ages and lists longer than the gameplay admission limit of three; the
saved-data representation therefore does not impose that limit. General mutable
bee/entity data, nectar gathering and occupant simulation remain outside this
generated-data contract.

The protocol block-entity type is **34**. Client update data is an empty compound,
encoded as **`0a00`**; occupants stay server-side. Persisted hive entries appear in
the owning chunk's `map_chunk` payload, including the queued saved-chunk load path.
Deferred tick requests add no client packet fields.

## Deferred requests and `.bcc` v4

`tick_request::TickRequest` stores an absolute block position, a `TickTarget` and
an `i32` relative delay. Feature requests have **NORMAL priority (0)**. Block
targets represent native block identity using its default-state ID; fluid targets
use `BuiltInRegistries.FLUID` IDs. Storage checks chunk ownership, Y **-64..319**,
block-state range **0..29873** and fluid range **0..5** (upper bounds exclusive).
Callers supply canonical target identities; storage does not normalize states.
It also does not cancel a request when the current block changes.

The format in `crates/bcore-protocol/src/chunk_store.rs` now writes version **4**
and reads **1, 2, 3 and 4**:

| Addition | Encoding |
|---|---|
| Beehive block entity | Kind `3`, `u32` occupant count, then one `i32` age per occupant |
| Generated sculk block entity | Kind `4`, `u32` JSON byte length, then `FeatureBlockEntity` including typed full NBT, update data, type and valid state range |
| Deferred requests, after structure data | `u32` count, then 14 bytes per request: `u8` kind (`1` block / `2` fluid), `u8` packed local X/Z, `i32` Y, `u32` target ID, `i32` delay |
| Unconsumed postprocessing marks | Header flag bit `0`; after requests: `u32` count, then `u8` packed local X/Z and `i32` Y per mark |

Multi-byte fields are little-endian and remain covered by the file checksum.
The reader validates kinds, counts, compatible hive states, coordinate reconstruction
and trailing data. Versions 1–3 load with no tick requests; kind `3` hives require
v4. Existing v3 minecart/structure records retain their layout and generated-entity
identity contract.

The postprocessing extension is emitted only when marks are present. Existing v4
files with flags `0` remain readable. Unknown flags, invalid mark positions/counts,
duplicate block-entity coordinates and incompatible generated sculk records are
rejected. Marks retain insertion order and duplicates, even if the block later
changes; storage does not execute them.

### Generated sculk metadata

`FeatureBlockEntity` retains both ordinary JSON and the native typed tree
`[NBT tag ID, payload]`. Sensor/shrieker `listener.selector.tick` therefore stays
`TAG_Long`, while delay, frequency and warning fields keep their native integer
types. The current codec validates **generated defaults**, not mutable gameplay
NBT, against the pinned sculk factory, state range and absolute owning position.

Native types **35** (sensor), **37** (catalyst) and **38** (shrieker) send an empty
chunk update compound, `0a00`; full data stays server-side. All **106 compatible
states** from `sculk_states_26_1.json` are tested through `.bcc` save/load/save and
chunk packet encoding. Tests additionally cover mixed hive/sculk records, exact
NBT widths, state replacement, corrupt metadata with valid checksums, marks and
tick duplicates. Actual generated sculk and deferred effects are checked through
`GenerationWorld` → `ChunkColumn` → `.bcc` → packet encoding.

These persisted requests retain their original delays and duplicates. They are
distinct from native `SavedTick` records and from an active level tick queue.

## Native tick-container semantics

[`TickReference.java`](../scripts/TickReference.java) calls native `ProtoChunkTicks`,
`LevelChunkTicks`, `SavedTick`, `ScheduledTick` and their codecs/comparators.
[`ticks_26_1.json`](../crates/bcore-worldgen/data/ticks_26_1.json) contains:

- **22 container traces:** three proto and 19 level cases;
- **4 request-preparation cases**;
- **24 signed conversion cases**, **81 comparator/identity pairs**;
- **12 chunk filters** and **11 priority/clamping cases**.

The Rust implementations live in `crates/bcore-worldgen/src/tick_queue.rs`:

1. **Preparation:** `PreparedTickQueues` keeps block and fluid identities separate.
   `ProtoTickQueue::schedule` deduplicates by position/type, keeps the first
   priority, and saves delay **0**, discarding the incoming trigger/sub-order.
   Native proto **load** instead retains the first saved delay/priority. Packing
   preserves insertion order. The original raw request list can remain intact.
2. **Level containers:** `LevelTickQueue::from_saved` retains pending saved entries,
   including duplicates. A one-time `unpack(game_time)` adds delays with Java long
   wrapping and assigns sub-orders **-N through -1**. Live scheduling uses
   position/type identity; saved duplicates retain the native membership behavior.
3. **Ordering and saves:** polling follows trigger tick, priority, then sub-order.
   Heap iteration/packing and comparator ties follow Java's priority queue rather
   than a coordinate sort. Saving subtracts game time as a wrapping long and
   narrows the result to a signed int. Priorities clamp to **-3..3**.
4. **Filtering:** native saved-tick chunk filtering checks X/Z only, preserving
   input order and duplicates even for out-of-height Y. The storage adapter's
   build-height validation above is a separate boundary.

These helpers currently have fixture-test callers, with no production handoff
from generated/loaded `.bcc` requests. `poll` returns work without checking game
time or executing it. Runtime integration still needs due-time/admission checks,
the world clock and sub-order allocation, cross-chunk `LevelTicks` coordination,
state/type checks, and actual block/fluid callbacks. The probe does not invoke
`ChunkMap.prepareTickingChunk` or `postProcessGeneration`.

## Verification and reproduction

On **2026-09-29**, independent native recapture with `--verify` passed for both
fixtures, the 18 rewrite traces and the feature-dependency graph. The full worldgen
suite passed **182 tests with 2 ignored**. Targeted checks include:

- **28 region tests**, including eleven effect-transfer tests, five standing-tree
  tests and the matching mineshaft-region test;
- **8 tick-container reference tests**;
- **7 protocol tree-effect tests** and **18 chunk-store tests**;
- **14 adjacent protocol integration tests**: flat format (5), vanilla chunk
  encoding (3), dungeon entities (2), entity packets (1), mineshaft entities (2)
  and queued entity delivery (1).

From the repository root, with `BCORE_DATAPACK` unset:

```powershell
cargo test --release --locked -p bcore-worldgen --lib region:: -- --test-threads=4
cargo test --release --locked -p bcore-worldgen --test tick_queue_reference
cargo test --release --locked -p bcore-protocol --lib chunk_store:: -- --test-threads=4
cargo test --release --locked -p bcore-protocol --test tree_effects --test chunk_format --test chunk_parity_vanilla --test dungeon_block_entities --test entity_packets_reference --test mineshaft_entities --test queued_entity_delivery -- --test-threads=4
```

Native recapture requires the pinned 26.1 JAR/libraries under `target/vanilla-775`,
`javac` 21+ and Java 25+. Use a unique build directory/output per concurrent run:

```powershell
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe beehive --build-dir target/native-probes/beehive --output target/beehives-reference.json --verify crates/bcore-worldgen/data/beehives_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe tree_effect --build-dir target/native-probes/tree-effect --output target/tree-effects-reference.json --verify crates/bcore-worldgen/data/tree_effects_26_1.json
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe tick --build-dir target/native-probes/tick --output target/ticks-reference.json --verify crates/bcore-worldgen/data/ticks_26_1.json
```

`--verify` compares the entire parsed fixture, including JAR and probe hashes.
The JAR SHA-256 is
`a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`.
Probe hashes concatenate raw source bytes in the runner's dependency order:
`TreeReference.java`, then `NativeWorldgenRegistries.java`, then
`BeehiveReference.java` for hives; `TreeReference.java`, then `TickReference.java`
for ticks. Rewrite traces hash `TreeReference.java`, `NativeEntityLevel.java`,
`NativeWorldgenRegistries.java`, then `TreeEffectReference.java`.
Adjust `--java`, `--javac` or `--vanilla` for the local installation.
