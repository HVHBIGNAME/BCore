"""Count total block states and report the direct-palette bit width vanilla uses.

The `map_chunk` block container falls back to the *global* (direct) palette when
a section has too many distinct states for a local palette. The bit width is
`ceil(log2(total_block_states))`, so BCore needs the exact registry size to emit
a byte-compatible direct container.

Run:  python scripts/count_block_states.py
"""

from __future__ import annotations

import json
import math
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
BLOCKS = ROOT / "target" / "datagen" / "reports" / "blocks.json"


def main() -> None:
    blocks = json.loads(BLOCKS.read_text(encoding="utf-8"))
    ids = set()
    for entry in blocks.values():
        for state in entry["states"]:
            ids.add(state["id"])

    total = len(ids)
    lo, hi = min(ids), max(ids)
    bits = math.ceil(math.log2(total))
    print(f"blocks          : {len(blocks)}")
    print(f"block states    : {total}")
    print(f"id range        : {lo}..{hi}")
    print(f"contiguous      : {ids == set(range(lo, hi + 1))}")
    print(f"direct palette  : {bits} bits (ceil_log2({total}))")
    print()
    print(f"    pub const BLOCK_STATE_COUNT: usize = {total};")
    print(f"    pub const DIRECT_PALETTE_BITS: u8 = {bits};")


if __name__ == "__main__":
    main()
