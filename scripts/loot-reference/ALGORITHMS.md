# Minecraft 26.1 native chest loot contract

Artifact: `target/vanilla-775/versions/26.1/server-26.1.jar`, SHA-256
`a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`.

This is an implementation-oriented reading of the shipped bytecode, corroborated
by actual-server observations where stated. Bytecode listings and their class
hashes are in `target/loot-native-20261016-bytecode-{core,functions,context,components}/`.
Each directory's `provenance.json` binds class bytes, disassembly bytes, JAR,
`javap` executable, command, and version. The class files are major version 69.

## Unpack and persistence

The implementation of `unpackLootTable(Player)` is the **default method on
`net.minecraft.world.RandomizableContainer`**, inherited by
`RandomizableContainerBlockEntity`. In order:

1. Read level, block position, and loot-table resource key.
2. If the key, level, or level's server is null, return without filling.
3. Resolve through `MinecraftServer.reloadableRegistries().getLootTable(key)`.
   Missing registry entries resolve to native `LootTable.EMPTY`.
4. For an actual `ServerPlayer`, trigger `CriteriaTriggers.GENERATE_LOOT` with the
   original key **before** clearing it.
5. Set the loot-table key to null. The `lootTableSeed` field is not changed.
6. Build `LootParams` with the original `ServerLevel` and
   `ORIGIN = Vec3.atCenterOf(blockPos)`, i.e. `(x+0.5,y+0.5,z+0.5)`.
7. When the player is nonnull, set luck from `Player.getLuck()` and set
   `THIS_ENTITY` to that player. Null-player builder luck is positive float zero.
8. Create parameters against `LootContextParamSets.CHEST`; call the resolved
   table's `fill(this, params, lootTableSeed)`.

Clearing the key before fill is essential: native `getItem` and `setItem` call
`unpackLootTable(null)` again. Those nested calls must become no-ops, not another
loot generation. `isEmpty`, `removeItem`, and `removeItemNoUpdate` also unpack.
Menu creation checks `canOpen` and then unpacks using the inventory's player.
A spectator cannot open a still-deferred loot container through that path.

`trySaveLootTable` writes `LootTable` and writes `LootTableSeed` only when nonzero.
While a key exists, chest saving uses that deferred representation rather than
the backing item list. After unpacking the key is null, so native chest saving
writes the materialized slots. A retained nonzero seed field alone is not a
reason to write a deferred loot table. Load defaults a missing seed to zero.
Component collection adds `minecraft:container_loot` only while a key exists.

Evidence: `core/RandomizableContainer`, `core/RandomizableContainerBlockEntity`,
`core/ChestBlockEntity`, `context/BaseContainerBlockEntity` listings (full class
names are used as filenames). Native fixtures include before/after saved chest
NBT, backing slots, retained seed, null key, and a second unpack with no changes.
The player/advancement/spectator branches have static authority only in this track.

## RNG selection and arithmetic

`LootContext.Builder.withOptionalRandomSeed(s)` sets a new
`RandomSource.create(s)` **only if `s != 0`**. In this JAR that factory constructs
`LegacyRandomSource`. The builder's `create(randomSequence)` chooses:

1. its explicit random source, if present;
2. `MinecraftServer.getRandomSequence(sequenceIdentifier)`, if the table declares
   a random sequence;
3. `Level.getRandom()` otherwise.

The lambda targets are resolved by verbose disassembly in
`bytecode-components/...LootContext$Builder.javap.txt`, `BootstrapMethods`.
The server owns `RandomSequences` and passes the world-options seed to its `get`.
Its named sequences use `XoroshiroRandomSource`, not the explicit-seed LCG.
Seed zero is therefore **not** equivalent to an explicitly seeded LCG with seed
zero. Seed-zero sequence creation, persistence, and continuation are not covered
by the nonzero-seed native fixtures.

For explicit seeds, the exact LCG is:

```text
initial_state = (signed_i64_seed XOR 25214903917) AND ((1<<48)-1)
next(bits):
    state = (state * 25214903917 + 11) AND ((1<<48)-1)
    return signed_i32(state >> (48-bits))
```

* `nextInt(n)` requires `n>0`. Powers of two use `(n * next(31)) >> 31`.
  Other bounds use rejection of signed-i32-negative
  `bits - (bits % n) + (n-1)`. Do not replace it with a modulo-only sampler.
* `nextLong()` is `(signed_i64(next(32)) << 32) + signed_i64(next(32))`, with
  Java wrapping arithmetic. The low word is **sign extended**, not ORed unsigned.
* `nextBoolean()` consumes `next(1)`.
* `nextFloat()` consumes `next(24)` and multiplies by `2^-24` in float32.
* `nextDouble()` consumes 26 bits then 27 bits, concatenates them, and multiplies
  by `2^-53` in float64.
* `Mth.nextInt(r,min,max)` returns `min` **without drawing** if `min>=max`;
  otherwise it draws `r.nextInt(max-min+1)+min`.
* `Mth.nextFloat` likewise returns `min` without drawing for `min>=max`; its
  non-degenerate expression is float32 `nextFloat() * (max-min) + min`.

The oracle records primitive draws, their arguments/results, before/after 48-bit
states, Gaussian-cache bits, and selected public RNG operations. Continuation
is evaluated by native methods on a copy reconstructed from the observed state:
`nextInt(1000), nextLong(), nextBoolean(), nextFloat(), nextDouble()` in that order.
The original fill source is never advanced for this continuation observation.
`verify.py` independently checks each recorded LCG transition and continuation.

## `LootTable.fill`: ordered phases

The bytecode offsets below refer to `bytecode-core/...LootTable.javap.txt`.

1. Construct the context with optional seed and the root table's sequence (0–19).
2. Generate the complete ordered item list with that context (21–27).
3. Obtain its same RNG (29–34).
4. Scan slots `0..size-1`, select **only `isEmpty()` slots**, then shuffle that
   ascending index list (36–43). Existing compatible stacks are not merged.
5. Run `shuffleAndSplitItems(items, availableSlots.size, sameRng)` (45–57).
6. Walk the resulting item list in order. For each item, pop the **last** available
   slot and call the native container's `setItem`. If no slot remains, warn
   `Tried to over-fill a container` and return, discarding the remaining generated
   stacks (60–185). There is no early capacity check before generation or shuffle.

The raw generator applies table functions around pool functions around entry
functions. A nested loot table receives the **same context**, so its own
`random_sequence` does not reseed nested evaluation. It uses a visited-table guard
to detect recursion. Nested item effects flow from inner entry/pool/table
functions through the parent's entry/pool/table functions and then the root
stack splitter.

### Maximum-size splitter during item generation

`createStackSplitter(level, consumer)` returns the lambda at the end of the
`LootTable` listing. It first checks `stack.isItemEnabled(level.enabledFeatures())`.
Disabled items are dropped. If `count < stack.getMaxStackSize()`, it forwards the
original stack. Otherwise it forwards successive `copyWithCount(min(max, remaining))`
stacks until remaining count reaches zero. It consumes no RNG. Equality with the
maximum still takes the copy branch. This maximum-size split precedes slot shuffling
and is distinct from the randomized split below.

The built-in matrix observes the normal splitter calls and randomized splits;
arbitrary over-maximum counts and disabled-feature item branches are not exhaustive
runtime coverage. The full bytecode is retained for those branches.

### Randomized split and shuffle

All removals below are **order-preserving** (`remove(index)`), not `swap_remove`.
Both `done` and `pending` preserve insertion order.

```text
done = generated list
pending = []
walk done in original order:
    if stack.isEmpty(): remove it
    else if stack.count > 1: move it to pending

while slot_count - done.size - pending.size > 0 AND pending is nonempty:
    original = pending.remove(Mth.nextInt(rng, 0, pending.size-1))
    amount = Mth.nextInt(rng, 1, original.count / 2)
    separated = original.split(amount)  # original is reduced first
    if original.count > 1 AND rng.nextBoolean(): pending.append(original)
    else: done.append(original)
    if separated.count > 1 AND rng.nextBoolean(): pending.append(separated)
    else: done.append(separated)

done.append_all(pending)
Util.shuffle(done, rng)
```

The two booleans are short-circuited: no boolean draw when the respective count
is one. The selected-pending-index draw is skipped for a single pending stack,
and the amount draw is skipped when half-count is one, due to `Mth.nextInt`.

`Util.shuffle` is descending Fisher–Yates: for `i=size` down to `2`, draw
`j=rng.nextInt(i)` and swap `list[i-1]` with `list[j]`. Zero/one-element lists draw
nothing. This operation is used separately for empty-slot indices and the final
item list. Their order cannot be interchanged.

## Pools, entries, and providers

* Pool conditions are tested before rolls or functions. A failing pool generates
  nothing. The roll count is `rolls.getInt(ctx) + floor(bonusRolls.getFloat(ctx) *
  ctx.luck)`; bonus-roll provider evaluation still occurs at luck zero.
* For each roll, expand entries in declared order, retaining positive-weight
  results. Singleton weight is `max(floor(float32(weight) + float32(quality) *
  luck), 0)`. An empty entry can have positive selection weight and emit nothing.
* No results or zero total weight returns. Exactly one expanded result is emitted
  with **no weighted-selection draw**. For multiple results, draw
  `nextInt(totalWeight)` and subtract weights in order until the residual is
  negative. This single-result shortcut belongs to `LootPool`, not every weighted
  choice API.
* `NumberProvider.getInt` defaults to `Mth.floor(getFloat(ctx))`.
  `UniformGenerator.getInt` evaluates min's integer provider, then max's integer
  provider, then the inclusive `Mth.nextInt`. Its float form uses each endpoint's
  float provider and `Mth.nextFloat`.
* `SetItemCountFunction` sets `base + count.getInt(ctx)` where base is old count
  for `add=true`, zero otherwise. It does not itself enforce maximum stack size.
* `LootItemRandomChanceCondition` compares a native `nextFloat()` to the chance
  provider. Preserve float32 evaluation and strict comparison.

## Component-changing functions

Use exported registry/tag order, item defaults, and the table JSON. The complete
function listings are retained; the following are the important integration rules.

### Enchant randomly

Use the explicit options holder set in its native iteration order, or the native
enchantment registry stream. A plain book skips compatibility filtering; otherwise
`onlyCompatible` controls `canEnchant` filtering. `Util.getRandomSafe` chooses an
enchantment. The level is inclusive between its min and max, with the same
degenerate-interval no-draw rule. A plain book is replaced by a **new** enchanted
book stack before enchantment is applied. Enchantments on an enchanted book live
in `minecraft:stored_enchantments`, not `minecraft:enchantments`.

### Enchant with levels

Evaluate levels as an integer provider, then call native `EnchantmentHelper.enchantItem`.
Inside `selectEnchantment`:

1. Missing `minecraft:enchantable` returns an empty selection without RNG work.
2. Increase cost by `1 + nextInt(enchantability/4+1) + nextInt(enchantability/4+1)`.
3. Compute float32 span `(nextFloat()+nextFloat()-1)*0.15`, then float32
   `cost + cost*span`, Java `Math.round(float)`, and clamp to `[1, INT_MAX]`.
4. In source order, retain primary-item-compatible enchantments (or any for a
   plain book). For each, walk levels downward from max to min, keeping the first
   whose inclusive min/max cost range contains the adjusted cost.
5. Choose from the candidates using `WeightedRandom` and enchantment weights.
   This helper draws for a positive total **even with one candidate**.
6. While `nextInt(50) <= cost`, remove candidates incompatible with the last
   selection. Stop if none remain; otherwise make another weighted choice and
   halve cost with integer division. The loop-condition draw occurs before that
   emptiness check.
7. The enclosing `enchantItem` replaces a plain book with a new enchanted book and
   applies the selected enchantments in order.

`enchantment.json` includes definitions, primary/supported item sets and exclusive
sets; `items.json` includes enchantability and item tags. Native holder/tag order
is retained. Sorting candidate tags alphabetically would change results.

### Other chest functions

* `set_damage`: for a damageable item, let base be `1-currentDamage/maxDamage`
  for `add=true`, otherwise zero. Set damage to
  `floor((1-clamp(provider.getFloat()+base,0,1))*maxDamage)` using float32 at every
  arithmetic step. Nondamageable items take a warning/no-change branch.
* `set_potion`: update the existing potion-contents component using `withPotion`,
  with the empty potion-contents value as the default. This is not an item-ID swap.
* `set_instrument`: choose from the options holder set and store an
  `InstrumentComponent` under `minecraft:instrument`.
* `set_stew_effect`: only acts on suspicious stew with a nonempty effects list;
  randomly choose an effect, evaluate integer duration, multiply duration by 20
  unless the effect is instantaneous, then append/update the stew-effects
  component. Native effect identities and durations are in the fixtures.
* `set_ominous_bottle_amplifier`: evaluate its integer provider, clamp to `[0,4]`,
  and set `minecraft:ominous_bottle_amplifier`.
* `set_enchantments`: transmute a plain book to an enchanted book using
  `transmuteCopy` (different from the new-stack behavior of the two random
  enchantment functions). For each specified enchantment, set or add its integer
  provider value, clamping to `[0,255]`. Unmentioned enchantments are not cleared.

The two enchantment functions can additionally write
`minecraft:additional_trade_cost` when configured and when the context contains
`ADDITIONAL_COST_COMPONENT_ALLOWED`. That context branch is not represented by
the null-player CHEST fixtures.

## World-dependent and uncovered authority

`exploration_map` calls `ServerLevel.findNearestMapStructure` with the destination
tag, integer-containing origin, search radius, and skip-known flag. If a structure
is found, it creates a persistent map, renders a biome preview, and adds a target
decoration. A constant map ID, canned position, fake level, or empty context would
not be an oracle for that behavior. The three affected built-in chest tables
(`shipwreck_map`, `underwater_ruin_big`, `underwater_ruin_small`) are exported but
not evaluated by the bounded matrix. Their `set_name` paths are also not dynamically
covered here.

Other explicit gaps: seed-zero named sequence/level RNG and reload behavior;
real players, nonzero luck and advancement effects; double-chest/menu semantics;
minecart/dispenser/pot/vault invocation lifecycle (their candidate loot tables can
be evaluated in a chest without proving those lifecycles); arbitrary datapack
conditions/functions; feature-flag-disabled items; malformed/cyclic tables;
arbitrary component-removal and over-maximum-count controls. Fixture assertions
cover their recorded cases, not universal implementation completeness.

## Rust implementation boundary

A reusable evaluator needs an ordered decoded table graph, persistent item defaults
and patches, numeric providers, conditions/functions, native-order registry/tag
lookups, and the exact random-source abstraction. Keep the returned item stream
separate from container placement so both RNG checkpoints are testable. On the
runtime side, unpack owns key clearing, player context, persistence and the
container's real setter behavior.

Potential optimizations consistent with this evidence: decode tables once, resolve
keys/tags ahead of repeated fills, cache constant-luck weights, use compact vectors
for <=54 slots, and share immutable default component data. Preserve ordered
removals, function order, float32 rounding and all skipped/required RNG draws.
These are implementation suggestions, not measured speedups. The instrumented
oracle is not a Java performance baseline.
