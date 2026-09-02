"""Derive the exact `fluidCount` and heightmap semantics from vanilla captures.

Reads `crates/bcore-protocol/data/vanilla_terrain_chunks.bin` (recorded by
`capture_terrain.py` from a real 26.2 overworld) and, for every section and every
column, checks candidate rules against the ground truth vanilla actually sent.

This replaces guessing:

* `fluidCount` — is it "water+lava blocks", "waterlogged too", or something else?
* the three heightmap kinds — exactly which blocks each one counts.

Run:  python scripts/analyze_terrain_capture.py
"""

from __future__ import annotations

import pathlib
import struct

ROOT = pathlib.Path(__file__).resolve().parent.parent
CAPTURE = ROOT / "crates" / "bcore-protocol" / "data" / "vanilla_terrain_chunks.bin"
BLOCKS_REPORT = ROOT / "target" / "datagen" / "reports" / "blocks.json"

SECTION_COUNT = 24
SECTION_VOLUME = 4096
SECTION_BIOMES = 64
MIN_Y = -64
HEIGHTMAP_BITS = 9

HM_WORLD_SURFACE = 1
HM_MOTION_BLOCKING = 4
HM_MOTION_BLOCKING_NO_LEAVES = 5


def parse_varint(b: bytes, at: int = 0) -> tuple[int, int]:
    r = 0
    for j in range(5):
        x = b[at + j]
        r |= (x & 0x7F) << (7 * j)
        if not (x & 0x80):
            return r, j + 1
    raise ValueError("varint")


class Reader:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.at = 0

    def varint(self) -> int:
        v, used = parse_varint(self.data, self.at)
        self.at += used
        return v

    def i16(self) -> int:
        v = struct.unpack_from(">h", self.data, self.at)[0]
        self.at += 2
        return v

    def i32(self) -> int:
        v = struct.unpack_from(">i", self.data, self.at)[0]
        self.at += 4
        return v

    def u8(self) -> int:
        v = self.data[self.at]
        self.at += 1
        return v

    def longs(self, n: int) -> list[int]:
        out = list(struct.unpack_from(f">{n}q", self.data, self.at))
        self.at += n * 8
        return out


def unpack(bits: int, longs: list[int], entries: int) -> list[int]:
    if bits == 0:
        return [0] * entries
    per_long = 64 // bits
    mask = (1 << bits) - 1
    out = []
    for i in range(entries):
        word = longs[i // per_long] & 0xFFFFFFFFFFFFFFFF
        out.append((word >> ((i % per_long) * bits)) & mask)
    return out


def read_container(r: Reader, entries: int) -> tuple[int, list[int], list[int]]:
    bits = r.u8()
    if bits == 0:
        single = r.varint()
        return bits, [single], [single] * entries
    palette = [r.varint() for _ in range(r.varint())]
    per_long = 64 // bits
    n_longs = (entries + per_long - 1) // per_long
    longs = r.longs(n_longs)
    return bits, palette, [palette[i] for i in unpack(bits, longs, entries)]


def decode(payload: bytes) -> dict:
    r = Reader(payload)
    x, z = r.i32(), r.i32()
    heightmaps = {}
    for _ in range(r.varint()):
        kind = r.varint()
        n = r.varint()
        heightmaps[kind] = unpack(HEIGHTMAP_BITS, r.longs(n), 256)

    data_len = r.varint()
    end = r.at + data_len
    sections = []
    while r.at < end:
        block_count = r.i16()
        fluid_count = r.i16()
        bits, palette, states = read_container(r, SECTION_VOLUME)
        read_container(r, SECTION_BIOMES)
        sections.append(
            {"block_count": block_count, "fluid_count": fluid_count, "states": states}
        )
    assert r.at == end
    return {"x": x, "z": z, "heightmaps": heightmaps, "sections": sections}


def packets(blob: bytes):
    count = int.from_bytes(blob[0:4], "big")
    at = 4
    for _ in range(count):
        pid = int.from_bytes(blob[at : at + 4], "big", signed=True)
        length = int.from_bytes(blob[at + 4 : at + 8], "big")
        at += 8
        yield pid, blob[at : at + length]
        at += length


def load_block_sets() -> dict[str, set[int]]:
    """State-id sets for the block families the heightmap rules care about."""
    import json

    blocks = json.loads(BLOCKS_REPORT.read_text(encoding="utf-8"))

    def ids(*names: str) -> set[int]:
        out: set[int] = set()
        for name in names:
            entry = blocks.get(name)
            if entry:
                out |= {s["id"] for s in entry["states"]}
        return out

    leaves = set()
    logs = set()
    plants = set()
    for name, entry in blocks.items():
        short = name.split(":", 1)[1]
        state_ids = {s["id"] for s in entry["states"]}
        if short.endswith("_leaves"):
            leaves |= state_ids
        if short.endswith("_log") or short.endswith("_wood"):
            logs |= state_ids
        if short in {
            "short_grass",
            "tall_grass",
            "fern",
            "large_fern",
            "dead_bush",
            "dandelion",
            "poppy",
            "blue_orchid",
            "allium",
            "azure_bluet",
            "red_tulip",
            "orange_tulip",
            "white_tulip",
            "pink_tulip",
            "oxeye_daisy",
            "cornflower",
            "lily_of_the_valley",
            "sunflower",
            "lilac",
            "rose_bush",
            "peony",
            "sugar_cane",
            "seagrass",
            "tall_seagrass",
            "kelp",
            "torch",
            "wall_torch",
            "snow",
            "vine",
            "glow_lichen",
            "brown_mushroom",
            "red_mushroom",
            "sweet_berry_bush",
            "wheat",
            "pumpkin_stem",
            "melon_stem",
            "fire",
            "rail",
            "powered_rail",
            "detector_rail",
            "activator_rail",
            "redstone_wire",
            "tripwire",
            "lily_pad",
            "bamboo_sapling",
            "cobweb",
            "pointed_dripstone",
            "hanging_roots",
            "cave_vines",
            "cave_vines_plant",
            "big_dripleaf_stem",
            "spore_blossom",
            "azalea",
            "moss_carpet",
            "pink_petals",
            "firefly_bush",
            "leaf_litter",
            "wildflowers",
            "bush",
            "cactus_flower",
            "short_dry_grass",
            "tall_dry_grass",
        }:
            plants |= state_ids
        if short.endswith("_sapling") or short.endswith("_button") or short.endswith("_pressure_plate"):
            plants |= state_ids

    return {
        "air": ids("minecraft:air", "minecraft:cave_air", "minecraft:void_air"),
        "water": ids("minecraft:water"),
        "lava": ids("minecraft:lava"),
        "leaves": leaves,
        "logs": logs,
        "plants": plants,
    }


def main() -> None:
    blob = CAPTURE.read_bytes()
    sets = load_block_sets()
    air = sets["air"]
    water = sets["water"]
    lava = sets["lava"]
    leaves = sets["leaves"]
    plants = sets["plants"]

    chunks = [decode(p) for _, p in packets(blob)]
    print(f"loaded {len(chunks)} terrain chunks from {CAPTURE.name}\n")

    # ---- fluidCount --------------------------------------------------------
    print("=== RULE CHECK: fluidCount ===")
    rules = {
        "water only": lambda s: s in water,
        "water + lava": lambda s: s in water or s in lava,
    }
    for label, rule in rules.items():
        mismatches = 0
        total = 0
        example = None
        for chunk in chunks:
            for i, sec in enumerate(chunk["sections"]):
                total += 1
                got = sum(1 for s in sec["states"] if rule(s))
                if got != sec["fluid_count"]:
                    mismatches += 1
                    if example is None:
                        example = (chunk["x"], chunk["z"], i, sec["fluid_count"], got)
        status = "MATCHES" if mismatches == 0 else f"{mismatches}/{total} mismatch"
        print(f"  fluidCount == count({label:<14}) -> {status}")
        if example:
            x, z, i, want, got = example
            print(f"      e.g. chunk ({x},{z}) section {i}: vanilla={want} rule={got}")

    # ---- blockCount --------------------------------------------------------
    print()
    print("=== RULE CHECK: blockCount ===")
    for label, rule in {
        "non-air": lambda s: s not in air,
        "non-air, non-fluid": lambda s: s not in air and s not in water and s not in lava,
    }.items():
        mismatches = 0
        example = None
        for chunk in chunks:
            for i, sec in enumerate(chunk["sections"]):
                got = sum(1 for s in sec["states"] if rule(s))
                if got != sec["block_count"]:
                    mismatches += 1
                    if example is None:
                        example = (chunk["x"], chunk["z"], i, sec["block_count"], got)
        status = "MATCHES" if mismatches == 0 else f"{mismatches} mismatch"
        print(f"  blockCount == count({label:<20}) -> {status}")
        if example:
            x, z, i, want, got = example
            print(f"      e.g. chunk ({x},{z}) section {i}: vanilla={want} rule={got}")

    # ---- heightmaps --------------------------------------------------------
    print()
    print("=== RULE CHECK: heightmaps (value = highest matching Y + 1 - MIN_Y) ===")

    def column_states(chunk: dict, x: int, z: int) -> list[int]:
        """Bottom-to-top state list for one column."""
        out = []
        for sec in chunk["sections"]:
            for y in range(16):
                out.append(sec["states"][y * 256 + z * 16 + x])
        return out

    def derived(chunk: dict, predicate) -> list[int]:
        values = []
        for z in range(16):
            for x in range(16):
                col = column_states(chunk, x, z)
                height = 0
                for i in range(len(col) - 1, -1, -1):
                    if predicate(col[i]):
                        height = i + 1
                        break
                values.append(height)
        return values

    candidates = {
        HM_WORLD_SURFACE: ("non-air", lambda s: s not in air),
        HM_MOTION_BLOCKING: (
            "non-air, non-plant (fluids count)",
            lambda s: s not in air and s not in plants,
        ),
        HM_MOTION_BLOCKING_NO_LEAVES: (
            "non-air, non-plant, non-leaves",
            lambda s: s not in air and s not in plants and s not in leaves,
        ),
    }

    for kind, (label, predicate) in candidates.items():
        bad = 0
        total = 0
        example = None
        for chunk in chunks:
            want = chunk["heightmaps"].get(kind)
            if want is None:
                continue
            got = derived(chunk, predicate)
            for idx, (w, g) in enumerate(zip(want, got)):
                total += 1
                if w != g:
                    bad += 1
                    if example is None:
                        example = (chunk["x"], chunk["z"], idx % 16, idx // 16, w, g)
        status = "MATCHES" if bad == 0 else f"{bad}/{total} columns differ"
        print(f"  kind {kind} ({label}) -> {status}")
        if example:
            cx, cz, x, z, w, g = example
            print(
                f"      e.g. chunk ({cx},{cz}) col ({x},{z}): vanilla={w} (y={w - 1 + MIN_Y}) "
                f"rule={g} (y={g - 1 + MIN_Y})"
            )

    # Do the three kinds ever actually differ? (Confirms leaves/plants matter.)
    print()
    print("=== Do the heightmap kinds differ in practice? ===")
    differing = 0
    for chunk in chunks:
        hm = chunk["heightmaps"]
        if hm.get(HM_WORLD_SURFACE) != hm.get(HM_MOTION_BLOCKING_NO_LEAVES):
            differing += 1
    print(f"  WORLD_SURFACE != MOTION_BLOCKING_NO_LEAVES in {differing}/{len(chunks)} chunks")
    differing = 0
    for chunk in chunks:
        hm = chunk["heightmaps"]
        if hm.get(HM_MOTION_BLOCKING) != hm.get(HM_MOTION_BLOCKING_NO_LEAVES):
            differing += 1
    print(f"  MOTION_BLOCKING != MOTION_BLOCKING_NO_LEAVES in {differing}/{len(chunks)} chunks")

    # ---- palette widths seen ----------------------------------------------
    print()
    print("=== Sea level / water surface (max water Y across the capture) ===")
    max_water_y = None
    for chunk in chunks:
        for si, sec in enumerate(chunk["sections"]):
            for i, s in enumerate(sec["states"]):
                if s in water:
                    y = MIN_Y + si * 16 + i // 256
                    if max_water_y is None or y > max_water_y:
                        max_water_y = y
    print(f"  highest water block in capture: y={max_water_y}")


if __name__ == "__main__":
    main()
