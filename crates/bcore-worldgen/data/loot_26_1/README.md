# Native 26.1 loot catalog

These files are byte-preserving promotions of the actual loaded vanilla 26.1
datapack/item/registry exports. `promotion.json` records every original SHA-256,
byte length, and portable destination. The server artifact is pinned there too.

* `tables.json`: 1,326 built-in table definitions. The compiler retains explicit
  diagnostics for definitions outside the supported interpreter vocabulary.
* `items.json`: 1,506 numeric item identities, effective default components,
  maximum stack sizes, and 207 ordered item tags.
* `enchantment.json`: 43 definitions and native-order holder tags.
* `instrument.json`: eight instrument identities and ordered holder tags.
* `potions.json`: 46 potion identities and native potion registry IDs.
* `registry-types.json`: component and loot registry type IDs.

The native oracle is separate, at
`../../tests/data/loot_26_1/loot-fill-26.1.json`. Production code never reads that
file. Array order in registry/tag exports is significant: do not sort it.

Reproduce promotion using the main repository's
`scripts/loot-reference/promote_runtime.py` through its shared
`scripts/run_generation_check.py` queue. The promoter verifies pinned input hashes
against the native manifest before writing, refuses different existing files,
and verifies readback. See the worktree-root `LOOT-HANDOFF.md` for the exact command
and source paths. Runtime loading and tests need only these portable files.
