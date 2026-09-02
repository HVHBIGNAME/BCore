"""Compare heightmap transmission order between the flat and terrain captures.

`tests/chunk_terrain.rs` found that the recorded *terrain* chunks list heightmap
kinds as [5, 4, 1] while BCore emits [1, 5, 4] — the order that reproduces the
recorded *flat* chunks byte-for-byte. Both captures come from the same official
26.2 server, so one of these must be wrong about what vanilla always does.

This script decodes the heightmap headers of every chunk in both captures and
prints the kind order, so the encoder can follow whatever vanilla actually does
rather than whichever capture was looked at first.

Run:  python scripts/compare_heightmap_order.py
"""

from __future__ import annotations

import pathlib
import struct

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "crates" / "bcore-protocol" / "data"
FLAT = DATA / "play_packets.bin"
WALK = DATA / "vanilla_walk_chunks.bin"
TERRAIN = DATA / "vanilla_terrain_chunks.bin"

MAP_CHUNK = 0x2D


def parse_varint(b: bytes, at: int = 0) -> tuple[int, int]:
    r = 0
    for j in range(5):
        x = b[at + j]
        r |= (x & 0x7F) << (7 * j)
        if not (x & 0x80):
            return r, j + 1
    raise ValueError("varint")


def packets(blob: bytes):
    count = int.from_bytes(blob[0:4], "big")
    at = 4
    for _ in range(count):
        pid = int.from_bytes(blob[at : at + 4], "big", signed=True)
        length = int.from_bytes(blob[at + 4 : at + 8], "big")
        at += 8
        yield pid, blob[at : at + length]
        at += length


def heightmap_order(payload: bytes) -> tuple[tuple[int, ...], tuple[int, ...]]:
    """Return (kinds in order, long-counts in order)."""
    at = 8  # skip x, z
    count, used = parse_varint(payload, at)
    at += used
    kinds = []
    lengths = []
    for _ in range(count):
        kind, used = parse_varint(payload, at)
        at += used
        n, used = parse_varint(payload, at)
        at += used + n * 8
        kinds.append(kind)
        lengths.append(n)
    return tuple(kinds), tuple(lengths)


def report(path: pathlib.Path, label: str) -> None:
    if not path.is_file():
        print(f"{label:<10} (missing: {path.name})")
        return
    orders: dict[tuple, int] = {}
    total = 0
    for pid, payload in packets(path.read_bytes()):
        if pid != MAP_CHUNK:
            continue
        total += 1
        kinds, lengths = heightmap_order(payload)
        orders[(kinds, lengths)] = orders.get((kinds, lengths), 0) + 1
    print(f"{label:<10} {total} map_chunk packets")
    for (kinds, lengths), n in sorted(orders.items(), key=lambda kv: -kv[1]):
        print(f"    kinds={list(kinds)}  longCounts={list(lengths)}  x{n}")


def main() -> None:
    print("kind 1 = WORLD_SURFACE, 4 = MOTION_BLOCKING, 5 = MOTION_BLOCKING_NO_LEAVES")
    print()
    report(FLAT, "flat")
    report(WALK, "flat-walk")
    report(TERRAIN, "terrain")
    print()
    print("If the orders differ between captures, the order is NOT fixed by the")
    print("protocol and the encoder must not hard-code one and assert on it.")


if __name__ == "__main__":
    main()
