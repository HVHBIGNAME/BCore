import subprocess, json
import numpy as np
from PIL import Image

BC = 'C:/coding/MINECRAFT/BCore'
AIR, WATER, LAVA = 0, 86, 102
half = 3  # 6x6 chunks = 96x96 blocks
N = 16 * 2 * half
hmap = np.zeros((N, N), dtype=np.int32)
water = np.zeros((N, N), dtype=bool)
for ci, cx in enumerate(range(-half, half)):
    for cj, cz in enumerate(range(-half, half)):
        s = subprocess.run([f'{BC}/target/release/examples/dump_chunk.exe',
                            '846692123413862008', str(cx), str(cz)],
                           capture_output=True, text=True, cwd=BC, timeout=120).stdout
        d = json.loads(s)
        states = d['states']
        for z in range(16):
            for x in range(16):
                top = -64
                w = False
                for y in range(319, -1, -1):
                    st = states[(y + 64) * 256 + z * 16 + x]
                    if st != AIR:
                        top = y
                        w = (st == WATER or st == LAVA)
                        break
                hmap[cj * 16 + z, ci * 16 + x] = top
                water[cj * 16 + z, ci * 16 + x] = w

lo, hi = int(hmap.min()), int(hmap.max())
span = max(1, hi - lo)
norm = ((hmap - lo) / span * 255).astype(np.uint8)
img = np.zeros((N, N, 3), dtype=np.uint8)
img[..., 0] = norm                    # R: low=dark, high=bright
img[..., 1] = (255 - norm // 3)       # G: inverse-ish
img[..., 2] = 0
img[water] = [28, 96, 200]            # water = blue
Image.fromarray(img, 'RGB').save(f'{BC}/target/heightmap_spawn.png')
print('saved', f'{BC}/target/heightmap_spawn.png', 'range', lo, hi, 'size', N)
