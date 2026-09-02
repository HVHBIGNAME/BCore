"""Dump the `minecraft:worldgen/biome` registry order out of the captured
config-phase packets, giving each biome's network id.

Biomes are a *datapack* registry: their ids are not in the datagen reports, they
are assigned by the order of entries in the clientbound `registry_data` packet
(0x07 in the configuration state) which vanilla sends during registry sync. That
capture already lives in `crates/bcore-protocol/data/config_packets.bin`.

Capture format (same as play_packets.bin):
    [u32 count]  then count * ( [i32 packet_id][u32 len][len bytes] )

`registry_data` payload:
    Identifier registryId
    varint     entryCount
    entryCount * ( Identifier entryId, bool hasData, [NBT data] )

Run:  python scripts/extract_biomes.py
"""

from __future__ import annotations

import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
CAPTURE = ROOT / "crates" / "bcore-protocol" / "data" / "config_packets.bin"

WANTED = [
    "minecraft:plains",
    "minecraft:forest",
    "minecraft:desert",
    "minecraft:ocean",
    "minecraft:river",
    "minecraft:beach",
    "minecraft:windswept_hills",
    "minecraft:snowy_plains",
    "minecraft:snowy_slopes",
    "minecraft:jagged_peaks",
    "minecraft:stony_peaks",
    "minecraft:taiga",
    "minecraft:frozen_ocean",
]


def read_varint(data: bytes, at: int) -> tuple[int, int]:
    value = 0
    shift = 0
    while True:
        byte = data[at]
        at += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            break
        shift += 7
        if shift > 35:
            raise ValueError("varint too long")
    # sign-extend to i32
    if value & 0x8000_0000:
        value -= 0x1_0000_0000
    return value, at


def read_string(data: bytes, at: int) -> tuple[str, int]:
    length, at = read_varint(data, at)
    return data[at : at + length].decode("utf-8"), at + length


def packets(blob: bytes):
    count = int.from_bytes(blob[0:4], "big")
    at = 4
    for _ in range(count):
        pid = int.from_bytes(blob[at : at + 4], "big", signed=True)
        length = int.from_bytes(blob[at + 4 : at + 8], "big")
        at += 8
        yield pid, blob[at : at + length]
        at += length


def skip_nbt(data: bytes, at: int) -> int:
    """Skip one network-NBT value (nameless compound root, no root name)."""
    tag = data[at]
    at += 1
    return skip_payload(data, at, tag)


def skip_payload(data: bytes, at: int, tag: int) -> int:
    if tag == 0:  # END
        return at
    if tag == 1:  # BYTE
        return at + 1
    if tag == 2:  # SHORT
        return at + 2
    if tag in (3, 5):  # INT, FLOAT
        return at + 4
    if tag in (4, 6):  # LONG, DOUBLE
        return at + 8
    if tag == 7:  # BYTE_ARRAY
        n = int.from_bytes(data[at : at + 4], "big", signed=True)
        return at + 4 + n
    if tag == 8:  # STRING
        n = int.from_bytes(data[at : at + 2], "big")
        return at + 2 + n
    if tag == 9:  # LIST
        item = data[at]
        n = int.from_bytes(data[at + 1 : at + 5], "big", signed=True)
        at += 5
        for _ in range(n):
            at = skip_payload(data, at, item)
        return at
    if tag == 10:  # COMPOUND
        while True:
            child = data[at]
            at += 1
            if child == 0:
                return at
            nlen = int.from_bytes(data[at : at + 2], "big")
            at += 2 + nlen
            at = skip_payload(data, at, child)
    if tag == 11:  # INT_ARRAY
        n = int.from_bytes(data[at : at + 4], "big", signed=True)
        return at + 4 + n * 4
    if tag == 12:  # LONG_ARRAY
        n = int.from_bytes(data[at : at + 4], "big", signed=True)
        return at + 4 + n * 8
    raise ValueError(f"unknown nbt tag {tag} at {at}")


def main() -> None:
    blob = CAPTURE.read_bytes()
    found = {}
    for pid, payload in packets(blob):
        try:
            registry, at = read_string(payload, 0)
        except (IndexError, UnicodeDecodeError, ValueError):
            continue
        if not registry.startswith("minecraft:"):
            continue
        try:
            count, at = read_varint(payload, at)
        except (IndexError, ValueError):
            continue
        if not 0 < count < 2000:
            continue
        try:
            entries = []
            for _ in range(count):
                name, at = read_string(payload, at)
                has_data = payload[at]
                at += 1
                if has_data:
                    at = skip_nbt(payload, at)
                entries.append(name)
        except (IndexError, UnicodeDecodeError, ValueError):
            continue
        if at != len(payload):
            continue
        found[registry] = entries
        print(f"packet 0x{pid:02x}  {registry:<40} {len(entries)} entries")

    print()
    biomes = found.get("minecraft:worldgen/biome")
    if not biomes:
        print("!! worldgen/biome registry not found; registries seen:")
        for name in sorted(found):
            print("   ", name)
        return

    print(f"=== minecraft:worldgen/biome — {len(biomes)} entries ===")
    index = {name: i for i, name in enumerate(biomes)}
    for name in WANTED:
        got = index.get(name)
        print(f"{name:<32} id={got}")

    print()
    print("=== RUST CONSTANTS ===")
    for name in WANTED:
        got = index.get(name)
        if got is None:
            continue
        const = name.split(":")[1].upper()
        print(f"    /// `{name}`\n    pub const {const}: u32 = {got};")

    print()
    print("=== SANITY: plains must be 40 (already used by the flat chunk) ===")
    print("plains =", index.get("minecraft:plains"))


if __name__ == "__main__":
    main()
