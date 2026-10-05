# Generated-entity streaming (26.1)

Generated chest minecarts now stream with their owning chunks on protocol **775**.
The implemented scope is static, immutable **GENERATED / pre-gameplay** entities:
initial placement and loot data, reconstructed identity, spawn delivery and
removal as a player's chunk view changes.

## Data and identity

`GeneratedEntity::ChestMinecart` in
`crates/bcore-worldgen/src/generated_entity.rs` carries an initial block position
and loot seed. Its entity position is the block center (`coordinate + 0.5`), its
native type ID is **25**, and its loot table is
`minecraft:chests/abandoned_mineshaft`. Generation and native fixtures compare this
pre-gameplay data independently of entity UUIDs.

`ChunkColumn` retains the ordered generated-entity rows. Their kind, owning-chunk
position and loot seed use the layout introduced in `.bcc` v3, alongside block
entities and structure metadata. The current writer emits **v4** and the reader
accepts **v1–v4**. V4 adds [hive occupants and deferred tick requests](tree-effects.md)
while retaining the v3 entity layout. Streaming adds no persisted UUID or runtime-ID
field.

`TrackedEntity` in `crates/bcore-protocol/src/entity.rs` supplies two identities:

- **Runtime ID:** a process-wide atomic allocator starts at **393**, reserving
  **392** for the local player. An ID remains shared while its entity record has
  an owner. Reconstructing a record after all owners release it allocates a new ID.
- **UUID:** the first 16 bytes of a domain-separated SHA-256 hash, with UUIDv8 and
  variant bits set. Inputs are the world seed, owning chunk coordinates, row
  ordinal, native entity type, initial block position and loot seed. This never
  consumes placement RNG. Identical rows at different ordinals have distinct
  identities; unchanged ordered rows and the same seed reconstruct the same UUID
  after reload or recreation.

This identity contract depends on immutable initial data and row order. Generic
persistent mutable-entity NBT identity, movement between chunks, changed inventory
and other gameplay state require additional persistence and simulation work.

## Ownership and cache behavior

`World` is a cloneable handle to `Arc<WorldInner>` and caches a `CachedChunk`
containing encoded chunk bytes and an
`Arc<[TrackedEntity]>`. Each active `PlayerView` holds the same entity array for
its delivered chunks. The per-world `EntityTracker` indexes those arrays with
`Weak` references keyed by chunk and reuses a live array when its ordered generated
data matches.

Payload-cache eviction therefore preserves identities held by active viewers.
Delivery-state tokens also own the exact arrays during a view handoff. After the
cache, views and any transfer token release an array, the weak index does not
retain its entities. Expired index entries are pruned on cache clearing and at the
insertion threshold. Cache publication is synchronized so concurrent misses share
one published entity array. This is distinct from the immutable pre-feature
terrain cache used by world generation.

### World-specific queued generation

Queue routing is fixed: each `GenerationJob` retains the requesting `World` and
dispatches through that instance. Cloned handles share its cache, tracker and
in-flight set; independently constructed worlds keep separate reservations, even
for the same coordinates. One dispatcher services the queue.

The job's RAII cleanup releases its reservation on completion, send failure,
discard or unwind, and the dispatcher continues after a failed job. The owning
handle keeps the world alive until the job is released. **Six queue tests passed**,
covering world/seed/store isolation, clone coalescing, ownership and failure cleanup.
The separate **26 world/lifecycle tests passed**. The complete **2026-10-02 protocol
run passed 234 tests**, including these queue and lifecycle checks.

## Packet ordering and delivery state

`PlayerView::stream_chunks_from` in `crates/bcore-protocol/src/world.rs` buffers a
stream operation in this order:

1. Send the updated view center.
2. For chunks retired by a teleport, send their tracked entity IDs in
   `remove_entities`, then unload the old chunks.
3. Send `chunk_batch_start`, the available `map_chunk` packets, and
   `chunk_batch_finished`.
4. Spawn the generated entities from those delivered chunks after the batch.
5. Remove entities and unload other chunks that left the view distance.

Delivered chunks are tracked so later ticks do not spawn their entities again.
Leaving a chunk removes only the entities tracked for that chunk in that view.
Teleporting moves the old loaded set into a retirement set; even a same-chunk
teleport removes/unloads before resending the chunk and spawning its entities.

The stream operation commits its loaded-chunk, tracked-entity and retirement
bookkeeping only after `write_all` succeeds. A failed write leaves delivery work
pending, and the production play loop ends the connection on a streaming error.
The guarantee is about server bookkeeping; a socket write can fail after
transmitting some bytes.

### Transferring a connection's view

The public coordinate-only `mark_loaded` API has been removed. Handoffs now use
the opaque, **non-Clone** `ChunkDeliveryState`:

- `previous.into_delivery_state()` consumes the previous `PlayerView` and moves
  its loaded chunks, pending retired chunks, exact tracked entity `Arc` arrays
  and next teleport ID into the token.
- `destination.adopt_delivery_state(token)` moves that state into the destination.
  Its own position and streaming settings determine subsequent chunk selection;
  the previous view's teleport-ID sequence continues.
- Adoption requires a destination with no loaded chunks, tracked entities or
  pending retirements, and which has not issued teleports. On rejection the
  destination is unchanged and `Err(token)` returns the entire token intact,
  retaining entity ownership and pending removals.

Both views must belong to the **same client connection**. This is a caller
contract; the token does not authenticate a connection. Retain or adopt the token
while that connection remains live so its outstanding removals are not lost.

## Native packet evidence

`scripts/EntityPacketReference.java` uses the actual 26.1 clientbound game codec,
native registry IDs and native metadata serializers. The independently verified
capture in `crates/bcore-protocol/data/entity_packets_26_1.json` now covers **27
packet samples**. Native capture and independent recapture with `--verify` passed.
The latest confirmed Rust run of
`crates/bcore-protocol/tests/entity_packets_reference.rs` also passed against all
27 samples:

| Packet | Confirmed samples | Scope |
|---|---:|---|
| Spawn (`0x01`) | 16 | 15 static constructor cases plus the actual `MinecartChest.getAddEntityPacket(ServerEntity)` pairing path; VarInt ID boundaries, negative/border positions and signed zero |
| Remove (`0x4d`) | 4 | Empty, single and multiple entity-ID lists |
| Metadata (`0x63`) | 1 | Non-default fields read from a real native chest minecart |
| Teleport (`0x7d`) | 6 | Absolute position, zero delta movement, three rotations and both on-ground values |

Spawn encodes zero velocity as **one byte of `LpVec3`**, followed by the three
zero angles and zero object data. Teleport includes three f64 position values,
three f64 delta-movement values, f32 yaw/pitch, the relative-fields bitset and the
on-ground flag. The implemented teleport helper uses zero deltas and an empty
relative-fields bitset. The fixture also confirms native type IDs: item **71**,
zombie **150**, cow **30**, chest minecart **25**.

A default native chest minecart has no non-default synchronized metadata, so the
current static generated cart requires no initial metadata packet. The metadata
sample deliberately changes native entity fields to check their wire encoding.
These checks cover the stated static/absolute packet paths, not minecart physics
or every velocity/relative-update combination.

The original **26-sample** baseline is retained. Its added 27th sample, labeled
`native_minecart_pairing`, obtains the spawn packet from the actual native chest
minecart paired with `ServerEntity`, and matches the Rust encoder.

The Rust reference test checks the frame length separately, then compares packet
ID and body bytes with the native capture. The native probe also decodes its
encoded packets to check packet-type round trips. Captures record the combined
probe-source hash and pinned JAR SHA-256:
`a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`.

## Lifecycle verification

The latest confirmed targeted `world::` run passed **26 tests**, including
**four new adoption regressions**. The original seven lifecycle tests in
`crates/bcore-protocol/src/world/entity_tests.rs` remain covered:

1. Spawn after the owning chunk batch, exactly once while loaded.
2. Remove the leaving chunk's entities on unload.
3. Same-chunk teleport removes/unloads before resend and respawn.
4. Multiple viewers retain one identity across payload eviction; after all owners
   release it, reconstruction gives a new runtime ID and the same UUID.
5. Unchanged v3 data reconstructs the UUID, duplicate row ordinals remain distinct,
   and changing the world seed changes identity.
6. Concurrent cache misses publish one shared entity array.
7. Failed writes do not commit chunk/entity delivery or discard pending retirement.

The four adoption regressions check:

1. Identity survives token handoff and cache eviction, with no duplicate spawn;
   failed unload writes retain ownership until a successful removal/unload.
2. Pending teleport retirements and the teleport-ID sequence survive handoff,
   including failed writes and remove-before-respawn ordering.
3. A destination with existing delivery or teleport state rejects adoption without
   changes, returning an intact token that can be adopted by a fresh destination.
4. Adoption after a failed initial write leaves chunk/entity delivery pending.

The broad `.gitignore` rule `world/` was narrowed to `/world/`, retaining
`/crates/*/world/` for crate-local saves. This makes the source test module
`crates/bcore-protocol/src/world/entity_tests.rs` visible to version control rather
than hiding it under the save-directory rule.

The old unused login UUID helpers were removed; the earlier targeted streaming
run had no compile warnings. The **178-pass protocol baseline predates these
changes**. The six queue tests and 26 world tests are targeted results; the newer
complete suite passed **234 protocol tests** within a **514-pass / 4-ignored**
workspace run. Historical measurements and current reproduction details are in
[parity-report.md](parity-report.md).

## Reproduce

From the repository root, with `BCORE_DATAPACK` unset. The Rust tests use bundled
fixtures and require neither a live Minecraft server nor Java:

```powershell
cargo test --release --locked -p bcore-protocol --lib world_state::queue_tests -- --test-threads=4
cargo test --release --locked -p bcore-protocol --lib world:: -- --test-threads=4
cargo test --release --locked -p bcore-protocol --test entity_packets_reference
cargo test --release --locked -p bcore-protocol --test mineshaft_entities
```

For independent native recapture, install `javac` 21+ and Java 25+, and provide the
pinned server JAR plus libraries under `target/vanilla-775`. Adjust `--java`,
`--javac` or `--vanilla` as needed. The shared capture runner's `--build-dir` option
isolates compiled Java classes; give each concurrent invocation a unique build
directory and output path:

```powershell
python scripts/capture_tree_reference.py --java target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe --probe entity_packet --build-dir target/native-probes/entity-packet --output target/entity-packets-reference.json --verify crates/bcore-protocol/data/entity_packets_26_1.json
```

`--verify` checks both provenance and samples. Without `--build-dir`, probes share
`target/tree-reference`; use that default only for sequential captures. The broader
regression commands and slow-target timings are in
[parity-report.md](parity-report.md#regression-suites).

## Remaining scope

Minecart motion/ticking, loot unpacking, inventory interaction, mutable-entity
persistence and spawner simulation remain incomplete. `finish_chunk` now transfers
staged hive snapshots and raw tick requests into their owning region chunks.
The returned chunk carries them through `.bcc` v4 save/load and queued loading;
hive entries send empty client update NBT while occupants and requests remain
server-side. [Tree effects and tick preparation](tree-effects.md) records the
native evidence, transfer/retry behavior and targeted persistence checks.

Native packet and lifecycle checks do not establish locations in complete
mixed-feature regions. Shared live generation and region vegetation are connected;
native source-history matching, persisted generation holders, other structures,
dimensions and runtime tick execution remain tracked in
[worldgen-parity-plan.md](worldgen-parity-plan.md). Recorded suite and snapshot
provenance are in [parity-report.md](parity-report.md).
