"""Deterministic numeric inputs; all expected outputs come from the native JAR."""
import math
import random
import struct
from zipfile import ZipFile
import json


def requests(jar):
    rng = random.Random(775)
    fractional = [[x, y, z] for x, y, z in [
        (0.0, 0.0, 0.0), (-0.0, -0.0, -0.0), (1.0, 1.0, 1.0),
        (-1.0, -1.0, -1.0), (-0.25, 0.5, 0.75), (15.999984741210938, 63.5, -16.0),
        (16.0, -64.0, 16.0), (-16.0, 319.0, -17.0),
        (29999984.25, 255.75, -29999984.5),
        (33554431.5, -63.125, -33554432.5), (33554432.0, 0.0, -33554432.0),
    ]]
    fractional += [[rng.randrange(-30000000, 30000000) / 16,
                    rng.randrange(-1024, 5120) / 16,
                    rng.randrange(-30000000, 30000000) / 16] for _ in range(64)]
    blocks = [[x, y, z] for x, y, z in [
        (0, -64, 0), (0, -63, 0), (1, -1, 1), (-1, 0, -1), (2, 1, 3),
        (3, 7, 2), (4, 8, 4), (7, 15, 8), (15, 62, 15), (16, 63, 16),
        (-16, 64, -17), (-17, 65, -16), (99, 120, -100), (1000, 127, 0),
        (-2000, 128, 3000), (1, 248, 2), (2, 255, 3), (3, 318, 1), (0, 319, 0),
        (29999983, 57, -29999981), (-29999983, -27, 29999981),
    ]]
    blocks += [[rng.randrange(-30000000, 30000000), rng.randrange(-64, 320),
                rng.randrange(-30000000, 30000000)] for _ in range(43)]
    fade = [[x, 0, 0] for x in [-1.0, -0.0, 0.0, 0.5, 1.0, 2.0]]
    for x in (0.0, 0.5, 1.0):
        fade += [[math.nextafter(x, -math.inf), 0, 0], [math.nextafter(x, math.inf), 0, 0]]
    fade += [[rng.getrandbits(24) / 2**24, 0, 0] for _ in range(256)]
    lerp = [[0, -0.0, -0.0], [1, -0.0, 0.0], [0.5, -1, 1], [1, 1e16, 1],
            [0, 1, 1e16], [0.125, -1e16, 1e16], [-1, 17, -19], [2, 17, -19]]
    lerp += [[rng.getrandbits(24) / 2**24,
              rng.randrange(-2**40, 2**40) / 256,
              rng.randrange(-2**40, 2**40) / 256] for _ in range(256)]
    result = {"points": {"fractional": fractional, "blocks": blocks, "fade": fade, "lerp": lerp}, "cases": []}

    def add(label, op, points, **fields):
        result["cases"].append({"id": label, "op": op, "points": points, **fields})

    add("scalar/smoothstep", "smoothstep", "fade")
    add("scalar/wrap", "wrap", "fractional")
    add("scalar/clamped_lerp", "clamped_lerp", "lerp")
    for precision in ("f32", "f64"):
        add(f"scalar/lerp/{precision}", "lerp", "lerp", precision=precision)
    with ZipFile(jar) as archive:
        prefix = "data/minecraft/worldgen/"
        noises = sorted(n[len(prefix + "noise/"):-5] for n in archive.namelist()
                        if n.startswith(prefix + "noise/") and n.endswith(".json"))
        densities = sorted(n[len(prefix + "density_function/"):-5] for n in archive.namelist()
                           if n.startswith(prefix + "density_function/") and n.endswith(".json")
                           and ("/overworld/" in n or "/" not in n[len(prefix + "density_function/"):-5]))
        router = json.loads(archive.read(prefix + "noise_settings/overworld.json"))["noise_router"]
    epsilon_f32 = struct.unpack("f", struct.pack("f", 1e-7))[0]
    rarity = [-1.0, 1.0]
    for boundary in (-0.75, -0.5, 0.0, 0.5, 0.75):
        rarity += [math.nextafter(boundary, -math.inf), boundary, math.nextafter(boundary, math.inf)]
    for seed in (0, 1, -1, 846692123413862008, -(2**63), 2**63 - 1):
        for label, scale, fudge in (("plain", 0.0, 0.0), ("smear", 0.5, -1.0),
                                    ("epsilon", 1e-8, (1.0 - epsilon_f32) * 1e-8)):
            add(f"improved/{label}/{seed}", "improved", "fractional", seed=seed, y_scale=scale, y_fudge=fudge)
        for first, amps in ((-3, [1.0]), (-7, [1.0, 0.0, 0.5, 2.0]),
                            (1, [1.0, 1.0, 1.0]), (-15, [1.0] * 16)):
            add(f"perlin/{first}/{seed}", "perlin", "fractional", seed=seed, first_octave=first, amplitudes=amps)
        for name in noises:
            add(f"normal/{name}/{seed}", "normal", "fractional", seed=seed, noise="minecraft:" + name)
        for label, xs, ys, xf, yf, smear in (("overworld", 0.25, 0.125, 80.0, 160.0, 8.0),
                                            ("isotropic", 1.0, 1.0, 80.0, 160.0, 4.0)):
            add(f"blended/{label}/{seed}", "blended", "blocks", seed=seed,
                xz_scale=xs, y_scale=ys, xz_factor=xf, y_factor=yf, smear=smear)
            add(f"blended_density/{label}/{seed}", "density", "blocks", seed=seed, function={
                "type": "minecraft:old_blended_noise", "xz_scale": xs, "y_scale": ys,
                "xz_factor": xf, "y_factor": yf, "smear_scale_multiplier": smear})
        for name in densities:
            add(f"density/{name}/{seed}", "density", "blocks", seed=seed, function="minecraft:" + name)
        for field in router:
            add(f"router/{field}/{seed}", "router", "blocks", seed=seed, field=field)
        add(f"noise_chunk/final_density/{seed}", "noise_chunk", "blocks", seed=seed)
        for marker in ("cache_all_in_cell", "cache_once", "cache_2d", "flat_cache", "interpolated"):
            add(f"noise_chunk/{marker}/{seed}", "noise_chunk", "blocks", seed=seed, function={
                "type": "minecraft:" + marker, "argument": {
                    "type": "minecraft:noise", "noise": "minecraft:spaghetti_2d",
                    "xz_scale": 1.0, "y_scale": 0.0 if marker in ("cache_2d", "flat_cache") else 1.0}})
        for mapper in ("type_1", "type_2"):
            for i, value in enumerate(rarity):
                add(f"weird/{mapper}/{i}/{seed}", "density", "blocks", seed=seed, function={
                    "type": "minecraft:weird_scaled_sampler", "input": value,
                    "noise": "minecraft:spaghetti_2d", "rarity_value_mapper": mapper})
    add("spline/float_knots", "density", "blocks", seed=0, function={
        "type": "minecraft:spline", "spline": {
            "coordinate": {"type": "minecraft:y_clamped_gradient", "from_y": -8, "to_y": 8,
                           "from_value": -1, "to_value": 1},
            "points": [{"location": x, "value": y, "derivative": d}
                       for x, y, d in [(-0.7, 0.3, -0.4), (-0.125, -0.7, 0.0), (0.0, 0.1, 0.8), (0.8, 0.9, 0.3)]]}})
    add("gradient/clamped_endpoints", "density", "blocks", seed=0, function={
        "type": "minecraft:y_clamped_gradient", "from_y": 0, "to_y": 8,
        "from_value": 999999.0, "to_value": 0.1})
    return result
