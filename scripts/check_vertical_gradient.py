"""Cross-check the vanilla vertical_gradient bedrock/deepslate seed path in pure Python."""

MASK48 = (1 << 48) - 1
M64 = 0xFFFFFFFFFFFFFFFF


def to_i64(v):
    v &= M64
    return v - (1 << 64) if v >= (1 << 63) else v


class JR:
    """java.util.Random / LegacyRandomSource."""

    def __init__(self, seed):
        self.s = (seed ^ 0x5DEECE66D) & MASK48

    def next(self, bits):
        self.s = (self.s * 0x5DEECE66D + 0xB) & MASK48
        return self.s >> (48 - bits)

    def next_long(self):
        hi = self.next(32)
        lo = self.next(32)
        hi = hi - (1 << 32) if hi >= (1 << 31) else hi
        return to_i64((hi << 32) + lo)

    def next_float(self):
        return self.next(24) / float(1 << 24)


def jhash(st):
    h = 0
    for c in st.encode():
        h = (h * 31 + c) & 0xFFFFFFFF
    return h - (1 << 32) if h >= (1 << 31) else h


def mth_get_seed(x, y, z):
    xm = (x * 3129871) & 0xFFFFFFFF
    xm = xm - (1 << 32) if xm >= (1 << 31) else xm
    i = to_i64(xm ^ to_i64(z * 116129781) ^ y)
    i = to_i64(to_i64(i * i) * 42317861 + i * 11)
    return i >> 16


SEED = 1234
fork = JR(SEED).next_long()
print("fork_seed:", fork)

# The parity chunk is (-34, 3) → world column x=-544, z=48.
WX, WZ = -544, 48

for name, below, above in (
    ("minecraft:bedrock_floor", -64, -59),
    ("minecraft:deepslate", 0, 8),
):
    named = to_i64(jhash(name) ^ fork)
    factory = JR(named).next_long()
    print(f"\n{name}: hash={jhash(name)} named={named} factory={factory}")
    for y in range(below + 1, above):
        prob = 1.0 - (y - below) / float(above - below)
        at = to_i64(mth_get_seed(WX, y, WZ) ^ factory)
        f = JR(at).next_float()
        print(f"  y={y:4} prob={prob:.3f} nextFloat={f:.4f} -> {f < prob}")
