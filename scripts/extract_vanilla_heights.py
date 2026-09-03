"""Extract vanilla WORLD_SURFACE heights from the terrain capture for parity."""
import sys
sys.path.insert(0, "scripts")
from analyze_terrain_capture import decode, packets, HEIGHTMAP_BITS, MIN_Y, HM_WORLD_SURFACE

CAPTURE = "crates/bcore-protocol/data/vanilla_terrain_chunks.bin"
blob = open(CAPTURE, "rb").read()
chunks = [decode(p) for _, p in packets(blob)]
print(f"loaded {len(chunks)} chunks")
for c in chunks:
    hm = c["heightmaps"].get(HM_WORLD_SURFACE)
    if hm is None:
        continue
    ys = [v - 1 + MIN_Y for v in hm]  # convert to absolute Y
    print(f"chunk ({c['x']},{c['z']}) surface Y: min={min(ys)} max={max(ys)} avg={sum(ys)//len(ys)}")
    print(f"    first 16 (x=0..15 at z=0): {ys[:16]}")
