#!/usr/bin/env python3
"""Replicate vanilla RNG (from decompiled bytecode) to cross-check BCore's
decoration seed + in_square positions for a chunk."""
import json

MASK = (1 << 64) - 1
SILVER = 0x6A09E667F3BCC909
GOLDEN = 0x9E3779B97F4A7C15

def rotl(x, k):
    return ((x << k) | (x >> (64 - k))) & MASK

class Xoroshiro:
    def __init__(self, lo, hi):
        self.lo = lo & MASK
        self.hi = hi & MASK
    def next_raw(self):
        l, m = self.lo, self.hi
        n = (rotl((l + m) & MASK, 17) + l) & MASK
        m2 = m ^ l
        self.lo = (rotl(l, 49) ^ m2 ^ ((m2 << 21) & MASK)) & MASK
        self.hi = rotl(m2, 28)
        return n

def mix_stafford13(z):
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
    return (z ^ (z >> 31)) & MASK

def upgrade(seed):
    lo = (seed ^ SILVER) & MASK
    hi = (lo + GOLDEN) & MASK
    return mix_stafford13(lo), mix_stafford13(hi)

def s64(x):
    return x - (1 << 64) if x >= (1 << 63) else x

class WorldgenRandom:
    def __init__(self, seed):
        self.src = Xoroshiro(*upgrade(seed))
    def next_long_raw(self):
        return self.src.next_raw()
    def next_bits(self, bits):
        return self.next_long_raw() >> (64 - bits)
    def next_int(self):
        return s64(self.next_bits(32) >> 0)  # next_bits(32) as i32 -> high 32 bits
    def next_long(self):
        i = self.next_bits(32)
        j = self.next_bits(32)
        # (long)i << 32 + (long)j, both sign-extended
        i_s = i - (1 << 32) if i >= (1 << 31) else i
        j_s = j - (1 << 32) if j >= (1 << 31) else j
        return ((i_s << 32) + j_s) & MASK
    def next_int_bounded(self, bound):
        if bound & (bound - 1) == 0:
            return (bound * s64(self.next_bits(31))) >> 31
        while True:
            sample = s64(self.next_bits(31))
            mod = sample % bound
            if sample - mod + (bound - 1) >= 0:
                return mod
    def set_decoration_seed(self, world_seed, bx, bz):
        self.src = Xoroshiro(*upgrade(world_seed))
        x_scale = (self.next_long() | 1)
        z_scale = (self.next_long() | 1)
        return ((bx * x_scale + bz * z_scale) ^ world_seed) & MASK

seed = 846692123413862008
r = WorldgenRandom(seed)
dec = r.set_decoration_seed(seed, 992, 0)
print("decoration_seed =", s64(dec), "(BCore: -3363180012525553896)")
# feature seed for index 43, step 9
feat = (dec + 43 + 10000 * 9) & MASK
r.src = Xoroshiro(*upgrade(feat))
count = 10 if r.next_int_bounded(10) < 9 else 11
print("count =", count)
pos = []
for _ in range(count):
    x = 992 + r.next_int_bounded(16)
    z = 0 + r.next_int_bounded(16)
    pos.append((x, z))
print("in_square xz:", pos)
