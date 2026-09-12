#!/usr/bin/env python3
"""Diagnose WHY tree-origin is 0%: is it x,z (seed) or y (height)?

Separately compares BCore and vanilla tree origins:
  - x,z overlap (ignoring y) -> did the FeatureSorter/seed fix take effect?
  - y delta distribution      -> is the residual a +/-1 terrain height error?
"""
from __future__ import annotations
import json, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BOT = ROOT / "scripts" / "bot" / "dump_terrain.js"
CHUNK = ROOT / "target" / "release" / "examples" / "dump_chunk.exe"
SEED = 846692123413862008
LOGS = {"oak_log", "birch_log", "spruce_log", "jungle_log", "acacia_log", "dark_oak_log"}
_IDS = {int(k): v for k, v in json.loads((ROOT/"scripts"/"block_ids.json").read_text()).items()}

def run(cmd, cwd=ROOT, timeout=240):
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, errors="replace", timeout=timeout)
    if p.returncode:
        raise RuntimeError(f"cmd failed ({p.returncode}): {' '.join(map(str,cmd))}\n{p.stderr[-1500:]}")
    return p.stdout

def vanilla_blocks(x, z, region, ymin, ymax):
    out, err = None, None
    p = subprocess.run(["node", str(BOT), "127.0.0.1", "25571", str(x), str(z), str(region), "bot",
                        "--full", f"--ymin={ymin}", f"--ymax={ymax}"],
                       cwd=BOT.parent, capture_output=True, text=True, errors="replace", timeout=300)
    if "position after teleport" not in p.stderr:
        raise RuntimeError("vanilla probe position not verified")
    data = json.loads(p.stdout)
    return {tuple(row[:3]): row[3] for row in data["blocks"]}

def bcore_blocks(x, z, region, ymin, ymax):
    out = {}
    half = region // 2
    for cx in range((x-half)//16, (x+half-1)//16+1):
        for cz in range((z-half)//16, (z+half-1)//16+1):
            raw = json.loads(run([str(CHUNK), str(SEED), str(cx), str(cz)]))
            states = raw["states"]
            for wx in range(max(x-half, cx*16), min(x+half, (cx+1)*16)):
                for wz in range(max(z-half, cz*16), min(z+half, (cz+1)*16)):
                    lx, lz = wx-cx*16, wz-cz*16
                    for y in range(ymin, ymax+1):
                        i = (y+64)*256 + lz*16 + lx
                        out[(wx, y, wz)] = _IDS.get(states[i], f"state_{states[i]}")
    return out

def origins(blocks):
    return {(x, y, z) for (x, y, z), n in blocks.items()
            if n in LOGS and blocks.get((x, y-1, z)) not in LOGS}

def main():
    x, z, region = 1000, 0, 16
    ymin, ymax = 40, 220
    print(f"sample ({x},{z}) region {region} ...")
    vb = vanilla_blocks(x, z, region, ymin, ymax)
    bb = bcore_blocks(x, z, region, ymin, ymax)
    vo, bo = origins(vb), origins(bb)
    print(f"vanilla origins: {len(vo)}   bcore origins: {len(bo)}")
    vxz = {(px, pz) for (px, _, pz) in vo}
    bxz = {(px, pz) for (px, _, pz) in bo}
    xz_overlap = vxz & bxz
    print(f"x,z overlap (seed match, ignore y): {len(xz_overlap)} / min={min(len(vxz), len(bxz))}")
    # y delta for the overlapping x,z
    from collections import Counter
    vy = {(px, pz): py for (px, py, pz) in vo}
    by = {(px, pz): py for (px, py, pz) in bo}
    deltas = Counter()
    for k in xz_overlap:
        deltas[by[k] - vy[k]] += 1
    print("y delta (bcore-vanilla) for x,z-overlapping trees:", dict(sorted(deltas.items())))
    # all vanilla xz positions for context
    print(f"vanilla xz count: {len(vxz)}, bcore xz count: {len(bxz)}")

if __name__ == "__main__":
    main()
