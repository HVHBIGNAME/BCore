#!/usr/bin/env python3
"""Brute-force the within-step feature index: which index makes BCore's
in_square positions match vanilla's actual trees at chunk (62,0)?"""
import json, subprocess, sys
sys.path.insert(0, '.')
from scripts.vanilla_rng_check import WorldgenRandom, s64, Xoroshiro, upgrade, MASK

seed = 846692123413862008
bx, bz = 992, 0

# vanilla's actual trees in chunk (62,0) z in [0,8) — from the earlier bot probe
vanilla_xz = {(995,1),(996,0),(996,1),(997,0),(997,1),(998,0),(999,1),(999,5),(1000,1),(1001,2),(1002,2)}

r = WorldgenRandom(seed)
dec = r.set_decoration_seed(seed, bx, bz)

best = []
for idx in range(0, 90):
    feat = (dec + idx + 10000 * 9) & MASK
    rr = WorldgenRandom.__new__(WorldgenRandom)
    rr.src = Xoroshiro(*upgrade(feat))
    # count weighted 10(w9)/11(w1)
    count = 10 if rr.next_int_bounded(10) < 9 else 11
    pos = set()
    for _ in range(count):
        x = bx + rr.next_int_bounded(16)
        z = bz + rr.next_int_bounded(16)
        pos.add((x, z))
    # how many of these positions land in the region z in [0,8) and match vanilla?
    region_pos = {(x, z) for (x, z) in pos if 0 <= z < 8}
    overlap = region_pos & vanilla_xz
    if overlap:
        best.append((len(overlap), idx, count, sorted(region_pos)))
    if len(overlap) >= 2:
        print(f"index={idx}: count={count}, overlap={len(overlap)}, positions={sorted(region_pos)}")

print("\nbest matches:", sorted(best, reverse=True)[:5])
