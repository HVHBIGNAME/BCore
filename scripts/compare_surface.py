#!/usr/bin/env python3
"""Diff BCore vs vanilla surface heights/blocks at the same world grid.

Runs the vanilla mineflayer bot and the BCore surface_grid example, parses
both `height:block` grids, and reports the height deltas and block agreement.
"""
import subprocess, sys, re

CENTER = sys.argv[1] if len(sys.argv) > 1 else "0"
CZ = sys.argv[2] if len(sys.argv) > 2 else "0"
REGION = sys.argv[3] if len(sys.argv) > 3 else "16"

# block-name map for the BCore u32 ids that appear at the surface
NAMES = {
    "9": "grass_block", "10": "dirt", "86": "water", "118": "sand",
    "1": "stone", "124": "gravel", "279": "oak_leaves", "335": "birch_leaves",
    "307": "spruce_leaves", "2248": "short_grass", "27846": "leaf_litter",
    "2250": "dead_bush", "143": "birch_log", "137": "oak_log", "140": "spruce_log",
}

def run(cmd, cwd=None):
    return subprocess.run(
        cmd, shell=True, capture_output=True, text=True, errors="replace", cwd=cwd
    ).stdout

def parse(text):
    grid = []
    for line in text.strip().splitlines():
        line = line.strip()
        if not line:
            continue
        row = []
        for cell in line.split():
            m = re.match(r"(-?\d+):(.+)", cell)
            if m:
                row.append((int(m.group(1)), m.group(2)))
        if row:
            grid.append(row)
    return grid

vanilla = parse(run(
    f'node dump_terrain.js 127.0.0.1 25571 {CENTER} {CZ} {REGION}',
    cwd='C:/coding/MINECRAFT/BCore/scripts/bot',
))
bcore = parse(run(
    f'C:/coding/MINECRAFT/BCore/target/release/examples/surface_grid.exe {CENTER} {CZ} {REGION}'
))

if len(vanilla) != len(bcore) or not vanilla:
    print(f"grid mismatch: vanilla {len(vanilla)} rows, bcore {len(bcore)} rows")
    sys.exit(1)

h = 0; h1 = 0; block_match = 0; n = 0; deltas = []
for vr, br in zip(vanilla, bcore):
    for (vy, vn), (by, bn) in zip(vr, br):
        n += 1
        d = by - vy
        deltas.append(d)
        if d == 0:
            h += 1
        if abs(d) <= 1:
            h1 += 1
        bname = NAMES.get(bn, bn)
        if bname == vn:
            block_match += 1

print(f"columns: {n}")
print(f"height exact match:  {h}/{n} = {100*h/n:.1f}%")
print(f"height within ±1:    {h1}/{n} = {100*h1/n:.1f}%")
print(f"top-block match:     {block_match}/{n} = {100*block_match/n:.1f}%")
print(f"height delta: min={min(deltas)} max={max(deltas)} avg={sum(deltas)/n:.2f}")
# histogram of deltas
from collections import Counter
print("delta histogram:", dict(sorted(Counter(deltas).items())))
