"""Diagnose host/Java math differences against immutable native misc observations."""

from collections import Counter
import json
import math
from pathlib import Path
import struct

from build_trig_table import make_table


ROOT = Path(__file__).resolve().parents[4]


def bits(value):
    return struct.unpack(">Q", struct.pack(">d", value))[0]


def number(value):
    return struct.unpack(">d", struct.pack(">Q", int(value)))[0]


def reduce_pio2(x):
    n = int(x * 6.36619772367581382433e-01 + 0.5)
    r = x - n * 1.57079632673412561417e+00
    w = n * 6.07710050650619224932e-11
    y0 = r - w
    exponent = (bits(x) >> 52) & 2047
    if exponent - ((bits(y0) >> 52) & 2047) > 16:
        t = r
        w = n * 6.07710050630396597660e-11
        r = t - w
        w = n * 2.02226624879595063154e-21 - ((t - r) - w)
        y0 = r - w
        if exponent - ((bits(y0) >> 52) & 2047) > 49:
            t = r
            w = n * 2.02226624871116645580e-21
            r = t - w
            w = n * 8.47842766036889956997e-32 - ((t - r) - w)
            y0 = r - w
    return n, y0, (r - y0) - w


def kernel_sin(x, y, tail):
    s1, s2, s3 = -1.66666666666666324348e-01, 8.33333333332248946124e-03, -1.98412698298579493134e-04
    s4, s5, s6 = 2.75573137070700676789e-06, -2.50507602534068634195e-08, 1.58969099521155010221e-10
    z = x * x
    v = z * x
    r = s2 + z * (s3 + z * (s4 + z * (s5 + z * s6)))
    return x - ((z * (0.5 * y - v * r) - y) - v * s1) if tail else x + v * (s1 + z * r)


def kernel_cos(x, y):
    c1, c2, c3 = 4.16666666666666019037e-02, -1.38888888888741095749e-03, 2.48015872894767294178e-05
    c4, c5, c6 = -2.75573143513906633035e-07, 2.08757232129817482790e-09, -1.13596475577881948265e-11
    z = x * x
    r = z * (c1 + z * (c2 + z * (c3 + z * (c4 + z * (c5 + z * c6)))))
    ix = (bits(x) >> 32) & 0x7fffffff
    if ix < 0x3fd33333:
        return 1.0 - (0.5 * z - (z * r - x * y))
    qx = 0.28125 if ix > 0x3fe90000 else number((ix - 0x00200000) << 32)
    hz = 0.5 * z - qx
    a = 1.0 - qx
    return a - (hz - (z * r - x * y))


def fd_sin_cos(angle):
    if abs(angle) <= math.pi / 4:
        return kernel_sin(angle, 0.0, False), kernel_cos(angle, 0.0)
    n, x, y = reduce_pio2(angle)
    s, c = kernel_sin(x, y, True), kernel_cos(x, y)
    return ((s, c), (c, -s), (-s, -c), (-c, s))[n & 3]


def table_sin_cos(angle, table):
    inverse = number(table["pi32_inverse"])
    p1, p2, p3 = map(number, table["pi32_split"])
    coefficients = [[number(v) for v in row] for row in table["coefficients"]]
    n = int(angle * inverse + math.copysign(0.5, angle))
    r1 = angle - n * p1
    r = r1 - n * p2
    negative_tail = n * p3 - ((r1 - r) - n * p2)
    r2 = r * r
    r4 = r2 * r2
    result = []
    for shift in (0, 16):
        c, hi, lo, sigma = map(number, table["table"][(n + shift) & 63])
        cosine = c + sigma
        ps = ((coefficients[1][0] * r2 + coefficients[0][0])
              + ((coefficients[3][0] * r1) * r + coefficients[2][0]) * r4) * ((cosine * r) * r2)
        pc = ((coefficients[1][1] * r2 + coefficients[0][1])
              + ((coefficients[3][1] * r1) * r + coefficients[2][1]) * r4) * (hi * r2)
        sr = sigma * r
        mid = c * r
        high = sr + hi
        total = mid + high
        correction = negative_tail * (hi * r - cosine) + lo
        correction += (hi - high) + sr
        correction += (high - total) + mid
        correction += ps
        correction += pc
        result.append(total + correction)
    return result


def main():
    fixture = json.loads((ROOT / "crates/bcore-worldgen/data/misc_feature_math_26_1.json").read_text(encoding="utf-8"))
    differences = Counter()
    examples = {}
    table = make_table()
    for sample in fixture["samples"]:
        angle = number(sample["angle"])
        sin, cos = fd_sin_cos(angle)
        table_sin, table_cos = table_sin_cos(angle, table)
        pairs = {
            "host_sin_vs_java": (bits(math.sin(angle)), int(sample["sin"])),
            "host_cos_vs_java": (bits(math.cos(angle)), int(sample["cos"])),
            "java_vs_strict_sin": (int(sample["sin"]), int(sample["strict_sin"])),
            "java_vs_strict_cos": (int(sample["cos"]), int(sample["strict_cos"])),
            "fd_sin_vs_java": (bits(sin), int(sample["sin"])),
            "fd_cos_vs_java": (bits(cos), int(sample["cos"])),
            "fd_sin_vs_strict": (bits(sin), int(sample["strict_sin"])),
            "fd_cos_vs_strict": (bits(cos), int(sample["strict_cos"])),
            "java_pow_vs_square": (int(sample["pow_x"]), int(sample["square_x"])),
            "table_sin_vs_java": (bits(table_sin), int(sample["sin"])),
            "table_cos_vs_java": (bits(table_cos), int(sample["cos"])),
        }
        for key, (a, b) in pairs.items():
            if a != b:
                differences[key] += 1
                examples.setdefault(key, {"angle": angle, "actual_bits": a, "native_bits": b})
    boundaries = json.loads((ROOT / "crates/bcore-worldgen/data/misc_feature_math_boundaries_26_1.json").read_text(encoding="utf-8"))
    for sample in boundaries["samples"]:
        angle = number(sample["angle"])
        s, c = table_sin_cos(angle, table)
        for key, a, b in [("boundary_sin", bits(s), int(sample["sin"])), ("boundary_cos", bits(c), int(sample["cos"]))]:
            if a != b:
                differences[key] += 1
                examples.setdefault(key, {"angle": angle, "actual_bits": a, "native_bits": b})
    print(json.dumps({"samples": len(fixture["samples"]), "differences": differences, "first_examples": examples}, indent=2))


if __name__ == "__main__":
    main()
