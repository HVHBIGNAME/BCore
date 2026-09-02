"""Extract network block-state ids + biome ids from the vanilla 26.2 datagen reports.

The reports are produced by:

    java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar --reports \
        --output target/datagen

`reports/blocks.json` lists, for every block, its property combinations and the
*network* block-state id vanilla assigns to each one (the same id space used by
`map_chunk` paletted containers). `reports/registries.json` carries the
`minecraft:worldgen/biome` registry, whose entry order is the biome network id.

Run:  python scripts/extract_block_states.py
"""

from __future__ import annotations

import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
REPORTS = ROOT / "target" / "datagen" / "reports"

# (rust const name, block id, required property subset for the default state)
WANTED: list[tuple[str, str, dict[str, str]]] = [
    ("AIR", "minecraft:air", {}),
    ("STONE", "minecraft:stone", {}),
    ("GRASS_BLOCK", "minecraft:grass_block", {"snowy": "false"}),
    ("DIRT", "minecraft:dirt", {}),
    ("COARSE_DIRT", "minecraft:coarse_dirt", {}),
    ("PODZOL", "minecraft:podzol", {"snowy": "false"}),
    ("BEDROCK", "minecraft:bedrock", {}),
    ("SAND", "minecraft:sand", {}),
    ("SANDSTONE", "minecraft:sandstone", {}),
    ("GRAVEL", "minecraft:gravel", {}),
    ("WATER", "minecraft:water", {"level": "0"}),
    ("LAVA", "minecraft:lava", {"level": "0"}),
    ("COAL_ORE", "minecraft:coal_ore", {}),
    ("IRON_ORE", "minecraft:iron_ore", {}),
    ("COPPER_ORE", "minecraft:copper_ore", {}),
    ("GOLD_ORE", "minecraft:gold_ore", {}),
    ("REDSTONE_ORE", "minecraft:redstone_ore", {"lit": "false"}),
    ("LAPIS_ORE", "minecraft:lapis_ore", {}),
    ("DIAMOND_ORE", "minecraft:diamond_ore", {}),
    ("DEEPSLATE", "minecraft:deepslate", {"axis": "y"}),
    ("OAK_LOG", "minecraft:oak_log", {"axis": "y"}),
    ("OAK_LEAVES", "minecraft:oak_leaves", {"distance": "1", "persistent": "false", "waterlogged": "false"}),
    ("BIRCH_LOG", "minecraft:birch_log", {"axis": "y"}),
    ("BIRCH_LEAVES", "minecraft:birch_leaves", {"distance": "1", "persistent": "false", "waterlogged": "false"}),
    ("SNOW_BLOCK", "minecraft:snow_block", {}),
    ("PACKED_ICE", "minecraft:packed_ice", {}),
    ("SHORT_GRASS", "minecraft:short_grass", {}),
    ("CACTUS", "minecraft:cactus", {"age": "0"}),
    ("DEAD_BUSH", "minecraft:dead_bush", {}),
]

WANTED_BIOMES = [
    "minecraft:plains",
    "minecraft:forest",
    "minecraft:desert",
    "minecraft:ocean",
    "minecraft:river",
    "minecraft:beach",
    "minecraft:windswept_hills",
    "minecraft:snowy_slopes",
    "minecraft:jagged_peaks",
    "minecraft:stony_peaks",
    "minecraft:taiga",
    "minecraft:snowy_plains",
]


def pick_state(entry: dict, want: dict[str, str]) -> dict:
    """The state matching every wanted property, else the `default: true` one."""
    states = entry["states"]
    if want:
        for state in states:
            props = state.get("properties", {})
            if all(props.get(k) == v for k, v in want.items()):
                return state
    for state in states:
        if state.get("default"):
            return state
    return states[0]


def main() -> None:
    blocks = json.loads((REPORTS / "blocks.json").read_text(encoding="utf-8"))
    registries = json.loads((REPORTS / "registries.json").read_text(encoding="utf-8"))

    print("=== BLOCK STATES (network ids, minecraft:block_state space) ===")
    rows = []
    for const, block, want in WANTED:
        entry = blocks.get(block)
        if entry is None:
            print(f"!! MISSING BLOCK {block}")
            continue
        state = pick_state(entry, want)
        props = state.get("properties", {})
        rows.append((const, block, state["id"], props))
        print(f"{const:<16} {block:<28} id={state['id']:<6} props={props}")

    print()
    print("=== RUST CONSTANTS ===")
    for const, block, sid, _props in rows:
        print(f"    /// `{block}`\n    pub const {const}: u32 = {sid};")

    print()
    print("=== BIOMES (minecraft:worldgen/biome network ids) ===")
    biome_reg = registries.get("minecraft:worldgen/biome")
    if biome_reg is None:
        print("!! no worldgen/biome registry in registries.json")
        print("available:", sorted(registries)[:40])
    else:
        entries = biome_reg["entries"]
        for name in WANTED_BIOMES:
            got = entries.get(name)
            if got is None:
                print(f"!! MISSING BIOME {name}")
            else:
                print(f"{name:<32} protocol_id={got['protocol_id']}")

    print()
    print("=== SANITY (already-known ids from captured flat chunks) ===")
    known = {"AIR": 0, "GRASS_BLOCK": 9, "DIRT": 10, "BEDROCK": 85}
    for const, _block, sid, _props in rows:
        if const in known:
            ok = "OK" if known[const] == sid else "MISMATCH"
            print(f"{const:<16} datagen={sid:<6} capture={known[const]:<6} {ok}")


if __name__ == "__main__":
    main()
