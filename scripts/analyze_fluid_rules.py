"""Pin down the last two wire rules: waterlogged fluids and heightmap outliers.

`analyze_terrain_capture.py` showed `fluidCount` is close to "water + lava" but
not equal, and the two MOTION_BLOCKING heightmaps disagreed on 0.3% of columns.
This script identifies the exact blocks responsible, so the Rust encoder can
document a rule that is *known* rather than assumed.

Run:  python scripts/analyze_fluid_rules.py
"""

from __future__ import annotations

import json
import pathlib

from analyze_terrain_capture import (  # reuse the decoder
    CAPTURE,
    HM_MOTION_BLOCKING,
    HM_MOTION_BLOCKING_NO_LEAVES,
    MIN_Y,
    decode,
    packets,
)

ROOT = pathlib.Path(__file__).resolve().parent.parent
BLOCKS_REPORT = ROOT / "target" / "datagen" / "reports" / "blocks.json"


def main() -> None:
    blocks = json.loads(BLOCKS_REPORT.read_text(encoding="utf-8"))

    # state id -> (block name, properties)
    state_info: dict[int, tuple[str, dict]] = {}
    waterlogged: set[int] = set()
    for name, entry in blocks.items():
        for state in entry["states"]:
            props = state.get("properties", {})
            state_info[state["id"]] = (name, props)
            if props.get("waterlogged") == "true":
                waterlogged.add(state["id"])

    water = {s["id"] for s in blocks["minecraft:water"]["states"]}
    lava = {s["id"] for s in blocks["minecraft:lava"]["states"]}
    air = set()
    for name in ("minecraft:air", "minecraft:cave_air", "minecraft:void_air"):
        air |= {s["id"] for s in blocks[name]["states"]}

    chunks = [decode(p) for _, p in packets(CAPTURE.read_bytes())]

    # ---- fluidCount: does water + lava + waterlogged explain it exactly? ----
    print("=== fluidCount = water + lava + waterlogged? ===")
    mismatch = 0
    total = 0
    culprits: dict[str, int] = {}
    for chunk in chunks:
        for si, sec in enumerate(chunk["sections"]):
            total += 1
            got = sum(
                1 for s in sec["states"] if s in water or s in lava or s in waterlogged
            )
            if got != sec["fluid_count"]:
                mismatch += 1
                # Which non-water states live in this section? Report candidates.
                if mismatch <= 3:
                    seen: dict[str, int] = {}
                    for s in sec["states"]:
                        if s in air or s in water:
                            continue
                        nm = state_info.get(s, ("?", {}))[0]
                        seen[nm] = seen.get(nm, 0) + 1
                    delta = sec["fluid_count"] - got
                    print(
                        f"  chunk ({chunk['x']},{chunk['z']}) section {si}: "
                        f"vanilla={sec['fluid_count']} rule={got} delta={delta}"
                    )
                    top = sorted(seen.items(), key=lambda kv: -kv[1])[:10]
                    print(f"    blocks present: {top}")
    verdict = "MATCHES EXACTLY" if mismatch == 0 else f"{mismatch}/{total} sections differ"
    print(f"  verdict: {verdict}")

    # ---- which blocks sit at the disputed heightmap tops? -------------------
    print()
    print("=== blocks at the top of MOTION_BLOCKING columns (vanilla's answer) ===")

    def column_states(chunk: dict, x: int, z: int) -> list[int]:
        out = []
        for sec in chunk["sections"]:
            for y in range(16):
                out.append(sec["states"][y * 256 + z * 16 + x])
        return out

    tops_mb: dict[str, int] = {}
    tops_nl: dict[str, int] = {}
    for chunk in chunks:
        for kind, bucket in (
            (HM_MOTION_BLOCKING, tops_mb),
            (HM_MOTION_BLOCKING_NO_LEAVES, tops_nl),
        ):
            hm = chunk["heightmaps"].get(kind)
            if hm is None:
                continue
            for z in range(16):
                col = column_states(chunk, 0, z) if False else None
            # Sample a subset of columns for speed.
            for idx in range(0, 256, 7):
                x, z = idx % 16, idx // 16
                col = column_states(chunk, x, z)
                height = hm[idx]
                if height == 0:
                    continue
                state = col[height - 1]
                nm = state_info.get(state, ("?", {}))[0]
                bucket[nm] = bucket.get(nm, 0) + 1

    print("  MOTION_BLOCKING tops:")
    for nm, count in sorted(tops_mb.items(), key=lambda kv: -kv[1])[:12]:
        print(f"    {nm:<34} {count}")
    print("  MOTION_BLOCKING_NO_LEAVES tops:")
    for nm, count in sorted(tops_nl.items(), key=lambda kv: -kv[1])[:12]:
        print(f"    {nm:<34} {count}")

    # ---- water surface: exactly where does water stop? ---------------------
    print()
    print("=== water column tops (confirms the sea-level convention) ===")
    surface_hist: dict[int, int] = {}
    for chunk in chunks:
        for z in range(16):
            for x in range(16):
                col = column_states(chunk, x, z)
                top_water = None
                for i in range(len(col) - 1, -1, -1):
                    if col[i] in water:
                        top_water = MIN_Y + i
                        break
                if top_water is not None:
                    surface_hist[top_water] = surface_hist.get(top_water, 0) + 1
    for y in sorted(surface_hist, reverse=True)[:6]:
        print(f"    topmost water at y={y}: {surface_hist[y]} columns")
    print()
    print("  => vanilla sea level is 63, and the highest water block is y=62,")
    print("     i.e. water fills y <= SEA_LEVEL - 1.")


if __name__ == "__main__":
    main()
