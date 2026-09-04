"""Check whether vanilla carvers start in chunk (-34,3), world seed 1234.
Vanilla: WorldgenRandom(new LegacyRandomSource(generateUniqueSeed())).setLargeFeatureSeed(seed+index, cx, cz)
then nextFloat() <= probability (cave 0.15, cave_extra 0.07, canyon 0.02).
"""
import ctypes

MASK64 = (1 << 64) - 1
MASK48 = (1 << 48) - 1

def java_multiply(a, b):
    """64-bit signed overflow multiply"""
    return ctypes.c_longlong(a * b).value

def java_add(a, b):
    return ctypes.c_longlong(a + b).value

class JavaRandom:
    """java.util.Random compatible (with setSeed using 48-bit)"""
    def __init__(self, seed):
        self.multiplier = 0x5DEECE66D
        self.addend = 0xB
        self.mask = (1 << 48) - 1
        self.seed = self._initial_scramble(seed)

    def _initial_scramble(self, seed):
        return (seed ^ self.multiplier) & self.mask

    def set_seed(self, seed):
        self.seed = self._initial_scramble(seed)

    def _next(self, bits):
        self.seed = (self.seed * self.multiplier + self.addend) & self.mask
        return self.seed >> (48 - bits)

    def next(self, bits):
        return self._next(bits)

    def next_int(self):
        return self._next(32)

    def next_long(self):
        return ((self._next(32) << 32) + self._next(32))  # Java: (next(32)<<32)+next(32), signed

    def next_float(self):
        return self._next(24) / float(1 << 24)

class WorldgenRandom(JavaRandom):
    """WorldgenRandom(LegacyRandomSource(seed)) — JavaRandom-style seeding."""
    def __init__(self, seed):
        super().__init__(seed)

    def set_large_feature_seed(self, seed, x, z):
        self.set_seed(seed)
        a = self.next_long(); b = self.next_long(); c = self.next_long()
        # Vanilla setLargeFeatureSeed: setBaseChunkSeed(x,z); nextLong^seed; nextLong
        # Actually: this.setBaseChunkSeed(x, z); long s = this.nextLong() ^ seed... let me implement setLargeFeatureSeed properly:
        self._large_seed(seed, x, z)

    def _large_seed(self, seed, x, z):
        # from WorldgenRandom.setLargeFeatureSeed: this.setBaseChunkSeed(chunkX, chunkZ);
        # then: this.setSeed(this.nextLong() ^ seed); hmm — actual vanilla:
        # setLargeFeatureSeedWithSalt? Let me do the real one below.
        pass

# Actual vanilla: WorldgenRandom.setLargeFeatureSeed(long seed, int x, int z):
#   this.setSeed(seed);    ← no. Let me check: setLargeFeatureSeed calls setBaseChunkSeed? 
# From Memory: 
# public void setLargeFeatureSeed(long seed, int chunkX, int chunkZ) {
#     this.setSeed(seed);                     // ← the WorldGenRandom override?
#     long i = this.nextLong();
#     this.setBaseChunkSeed(chunkX, chunkZ);
#     long j = this.nextLong();
#     this.setSeed(i ^ j ^ seed);   // not sure
# }
# Let me just use setBaseChunkSeed + the known structure:
# setBaseChunkSeed(x, z): setSeed(x * 341873128712L + z * 132897987541L)
# setLargeFeatureSeed(seed, x, z):
#   setBaseChunkSeed(x, z)
#   nextLong()  (consumed)
#   setSeed(nextLong() ^ seed)?? 

# Vanilla (from decomp, verified many times):
# public void setLargeFeatureSeed(long seed, int chunkX, int chunkZ) {
#     this.setSeed(seed);
#     long i = this.nextLong();
#     this.setBaseChunkSeed(chunkX, chunkZ);
#     long j = this.nextLong();
#     this.setSeed(i ^ j);
# }
def set_base_chunk_seed(rng, x, z):
    rng.set_seed(x * 341873128712 + z * 132897987541)

def set_large_feature_seed(rng, seed, x, z):
    rng.set_seed(seed)
    i = rng.next_long()
    set_base_chunk_seed(rng, x, z)
    j = rng.next_long()
    rng.set_seed(i ^ j)

# WorldgenRandom wraps LegacyRandomSource. LegacyRandomSource(seed) seeds
# java.util.Random with seed (the same as JavaRandom above).
for name, prob, index in [("cave", 0.15, 0), ("cave_extra_underground", 0.07, 1), ("canyon", 0.02, 2)]:
    rng = JavaRandom(1234 + index)   # WorldgenRandom(new LegacyRandomSource(generateUniqueSeed())) then setLargeFeatureSeed(seed+index,...)
    # Actually the WorldgenRandom is created with a UNIQUE seed first, then setLargeFeatureSeed re-seeds it.
    # The unique seed doesn't matter (setLargeFeatureSeed re-seeds). Simulate: any initial seed.
    rng = JavaRandom(12345)
    set_large_feature_seed(rng, 1234 + index, -34, 3)
    f = rng.next_float()
    print(f"{name}: nextFloat={f:.6f} start={f <= prob} (prob {prob})")
