# Fossils, desert pyramids and deferred block entities

The October integration prioritizes **observable terrain differences**. Three
parallel workers isolated repairs against the published `cdc85f1` baseline;
their changes were then merged and tested together. All comparisons use the
original **Minecraft Java 26.1 / protocol 775** server and immutable native inputs.
Full generation parity remains incomplete.

## Combined checkpoint — 2026-10-08

The final merged workspace passes **662 tests, with 0 failures and 4 ignored**.
One frozen executable replays **88 requests / 14 native histories** with zero
differences in **8,650,752 block-state** and **135,168 biome** observations.
Source order, logical structures, ordered postprocessing and all **40 scored
light snapshots** also match. The audit confirms that the frozen sources match
the final workspace source files.

The remaining scored differences are **36 NBT fields** at an implicit FULL
boundary and **70 WG-map presence fields** across repeated snapshots. All 88
requests retain incomplete coverage. The table below reports the three new
histories on this same integrated executable.

[Per-request measurements and provenance](metrics/checkpoint-2026-10-08.json) ·
[Detailed verification report](parity-report.md#terrain-and-lifecycle-repairs--2026-10-08).

## Reproduced terrain differences

Seed: `846692123413862008`. Counts sum request snapshots, including air and repeats;
they are not counts of distinct world positions.

| Native history | Requests | Baseline differing blocks | Integrated fix |
|---|---:|---:|---:|
| Fossils, CARVERS/FEATURES/adjacency | 8 | 269 | 0 |
| Fossils, FULL/adjacency/repeat | 3 | 103 | 0 |
| Desert pyramids, FEATURES/FULL/adjacency/repeat | 8 | 7,627 | 0 |

### Fossils

The production dispatcher rejected `fossil_coal` and `fossil_diamonds`. The first
native witness is `(189,42,2348)`: a bone block from source chunk `(11,146)` remained
andesite in BCore. The lower fossil at source `(3,149)` also spills into `(3,150)`.

The implementation uses all **16 native templates**, the two native processor
lists, the caller's continuing RNG, retained `OCEAN_FLOOR_WG`, native rotations,
corner rejection, clipping and shape callbacks between the bone and ore passes.
Independent component captures cover **76 cases**, **4,850 ordered writes** and
**568 tick requests**. Real feature-boundary captures check all nine input/output
chunks, including RNG continuation, biomes, WG heightmaps and ordered marks.

Code: `crates/bcore-worldgen/src/fossil.rs`.
Evidence: `fossil_{assets,history,reference}_26_1.json` in the worldgen data directory;
reproduction probes: `scripts/fossil-reference/`.

### Desert pyramids

Real starts at chunks **(-271,-179)** and **(-273,-15)** exposed the absent family.
The repair includes admission, native slot **step 4 / index 1**, reference bounds,
all four orientations, foundations, chest flags, cellar rubble and archaeology.
The source-region RNG continues across referenced starts independently of the
feature RNG and positional archaeology streams.

Independent captures verify **62 admissions**, **137 placement passes** and
**98 reference comparisons**. The integrated eight-request history removes all
7,627 state differences, 13 structure differences, 43 BE fields and 513 light-byte
differences. Six scored light snapshots match. Its 12 WG-presence differences
remain due to pending FULL conversion.

Code: `crates/bcore-worldgen/src/structure/desert_pyramid.rs`.
Evidence: `desert_pyramid_26_1.json`, `desert_pyramid_catalog_26_1.json`;
reproduction probes: `scripts/desert-pyramid-reference/`.

## Deferred block-entity lifecycle

`WorldGenRegion.setBlock` creates pending `id="DUMMY"` data in a proto chunk.
A real block-entity lookup materializes it from the block state or saved NBT.
Merely writing the block does not invoke its factory. Templates with saved NBT
do invoke the load path; templates with absent NBT leave the entity pending.

BCore now keeps pending typed NBT separately from materialized entities. Chest,
dispenser, hive and suspicious-sand callbacks resolve the destination's pending
entity before assigning loot or occupants. Compatible nest↔hive rewrites retain
the occupants, including when loaded from typed saved data.

- Two independent native captures: **39 cases / 55 save-read cycles** each.
- `.bcc` **v5** preserves pending data; readers accept **v1–v5**.
- Previous v4 materialized records remain materialized on load.
- Chunk packets enumerate materialized entities only. Empty update compounds
  project to null NBT (`00`) in the chunk-packet entry; standalone update tags
  retain their compound representation.
- FULL-request snapshots execute a **BE-only materialization boundary**; the
  FULL stage itself remains pending. Unsupported materialization is an error
  and retains the pending batch rather than publishing a partial replacement.

The prior 69-request corpus had **103 eager-versus-DUMMY field differences**.
Combined replays remove those but expose **36 fields in the final LIGHT-history
snapshot**: Vanilla completed FULL asynchronously before that later LIGHT read.
BCore still holds the proto state. This is a real missing ticket/lifecycle
transition; the comparator continues to score it. Repeating LIGHT is not treated
as a substitute for FULL.

Code: `block_entity/pending.rs`, `region.rs`, `chunk/pending.rs` and `chunk_store.rs`.
Evidence: `deferred_block_entities_26_1_v1.json` and the portable original-capture
archive in `scripts/block-entity-lifecycle-reference/fixtures/`.

## Retained generation mobs

The pre-existing SPAWN work now has a real retained-region adapter. Accepted
proto-mob saves survive late errors, preserve independently generated UUIDs and
are not duplicated by repeated requests. Native comparisons cover **456 saves
across all 19 overworld CREATURE types**, actual LOAD and initial pairing packets,
including metadata, attributes and nonempty equipment.

LOAD can clamp health, instantiate a wolf's max-health attribute and restore head
rotation differently from constructor state. In particular, goats clamp head
yaw before body rotation is restored. Negative-age frogs also have a loaded baby
metadata flag despite their `isBaby()` method override. These behaviors are
captured and tested, rather than inferred from the original finalized object.

This remains **initial generated state**, not AI or ticking simulation. The
runtime input API exposes world clocks, difficulty, border and mob settings;
the server's live clock/settings updates are not yet wired to it. Native entity
entropy is independent of the world seed. Matched terrain history results do
not score live entity lifecycle completeness.

## Remaining work, in priority order

1. Implement missing terrain/structure families, including `desert_well`, and
   expand matched native samples across seeds and biomes.
2. Implement ticket-driven FULL conversion, retaining the transition for later
   lower-status reads, WG retirement and native postprocessing/tick boundaries.
3. Persist generation holders and hydrate saved neighbours without losing source
   history or incoming effects.
4. Finish live world-input updates, generic structure-entity factories, gameplay
   and Nether/End pipelines.

The older three-region **96.99%** measurement remains tied to its original
executable and capture history. It is not overwritten by these targeted results.
See [the parity report](parity-report.md) and [versioned metrics](metrics/README.md)
for the combined-build checkpoint.
