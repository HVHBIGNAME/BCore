"""Terrain-surface parity: highest terrain block per column, vanilla vs BCore.

Earlier ad-hoc checks compared `id2name.get(...)` (which yields namespaced names
like `minecraft:air`) against bare `'air'`, so *every* block counted as terrain
and both surfaces trivially landed at y=319. This script uses namespaced names
and an explicit non-terrain set, so the numbers mean what they say.
"""

import json
import pathlib
import subprocess
import sys
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import analyze_terrain_capture as dec  # noqa: E402

CHUNK = (-34, 3)
SEED = 1234

# Fluids, air and anything decorative that sits *on top* of the terrain column.
NON_TERRAIN_SUFFIXES = (
    "_leaves",
    "_log",
    "_sapling",
    "_mushroom",
    "_flower",
    "_bush",
    "_fern",
    "_grass",
    "_roots",
    "_vine",
    "_lichen",
    "_moss_carpet",
)
NON_TERRAIN_EXACT = {
    "minecraft:air",
    "minecraft:cave_air",
    "minecraft:void_air",
    "minecraft:water",
    "minecraft:lava",
    "minecraft:leaf_litter",
    "minecraft:short_grass",
    "minecraft:tall_grass",
    "minecraft:fern",
    "minecraft:large_fern",
    "minecraft:dead_bush",
    "minecraft:vine",
    "minecraft:snow",
    "minecraft:sugar_cane",
    "minecraft:seagrass",
    "minecraft:tall_seagrass",
    "minecraft:kelp",
    "minecraft:kelp_plant",
    "minecraft:lily_pad",
    "minecraft:cactus",
    "minecraft:pumpkin",
    "minecraft:melon",
    "minecraft:brown_mushroom",
    "minecraft:red_mushroom",
    "minecraft:sweet_berry_bush",
    "minecraft:bush",
    "minecraft:firefly_bush",
    "minecraft:pale_moss_carpet",
    "minecraft:moss_carpet",
}


def is_terrain(name):
    if name in NON_TERRAIN_EXACT:
        return False
    return not name.endswith(NON_TERRAIN_SUFFIXES)


def load_id_map():
    report = json.loads((ROOT / "target/datagen/reports/blocks.json").read_text(encoding="utf-8"))
    out = {}
    for name, entry in report.items():
        for state in entry.get("states", []):
            out[int(state["id"])] = name
    return out


def vanilla_states():
    data = (ROOT / "crates/bcore-protocol/data/vanilla_terrain_chunks.bin").read_bytes()
    for _, packet in dec.packets(data):
        chunk = dec.decode(packet)
        if (chunk["x"], chunk["z"]) == CHUNK:
            states = []
            for section in chunk["sections"]:
                states.extend(section["states"])
            return states
    raise SystemExit(f"chunk {CHUNK} not in capture")


def bcore_states():
    proc = subprocess.run(
        ["cargo", "run", "-q", "-p", "bcore-worldgen", "--example", "dump_chunk",
         "--", str(SEED), str(CHUNK[0]), str(CHUNK[1])],
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    return json.loads(proc.stdout)["states"]


def terrain_top(states, id2name):
    tops = {}
    for x in range(16):
        for z in range(16):
            tops[(x, z)] = -65
            for y in range(319, -65, -1):
                name = id2name.get(states[(y + 64) * 256 + z * 16 + x], "minecraft:air")
                if is_terrain(name):
                    tops[(x, z)] = y
                    break
    return tops


def main():
    id2name = load_id_map()
    van, bc = vanilla_states(), bcore_states()
    vt, bt = terrain_top(van, id2name), terrain_top(bc, id2name)

    deltas = [bt[k] - vt[k] for k in sorted(vt)]
    exact = sum(1 for d in deltas if d == 0)
    print(f"terrain surface: exact {exact}/256   "
          f"avg |dy| {sum(abs(d) for d in deltas) / 256:.2f}   "
          f"max |dy| {max(abs(d) for d in deltas)}")
    print("signed delta histogram (bcore - vanilla):")
    for d, n in sorted(Counter(deltas).items()):
        print(f"  {d:+4}: {n:4}")

    blocks = sum(1 for a, b in zip(van, bc) if a != b)
    print(f"\nblock parity: {98304 - blocks}/98304 ({(98304 - blocks) / 98304 * 100:.2f}%)")


if __name__ == "__main__":
    main()
