"""Derive pi/32 split constants and sine/cosine data from high-precision arithmetic.

This is mathematical input data, not a captured Minecraft-output fixture. The
optional check compares the independently calculated numbers with the public
OpenJDK 25 x86-64 table. No OpenJDK source is copied into the Rust implementation.
"""

import argparse
from decimal import Decimal, localcontext
import hashlib
import json
from pathlib import Path
import re
import struct


ROOT = Path(__file__).resolve().parents[4]


def bits(value):
    return struct.unpack(">Q", struct.pack(">d", value))[0]


def number(value):
    return struct.unpack(">d", struct.pack(">Q", value))[0]


def series(x, cosine=False):
    term = Decimal(1) if cosine else x
    result = term
    for n in range(1, 128):
        term *= -x * x / ((2 * n - 1) * (2 * n) if cosine else (2 * n) * (2 * n + 1))
        old = result
        result += term
        if result == old:
            return result
    raise ArithmeticError("Taylor series did not converge")


def round32(value):
    rounded = bits(float(value))
    return number(((rounded + (1 << 20) - 1 + ((rounded >> 21) & 1)) >> 21) << 21)


def make_table():
    with localcontext() as context:
        context.prec = 120
        # Chudnovsky, evaluated with enough guard digits for binary64 residuals.
        m, l, x, k = 1, 13591409, 1, 6
        total = Decimal(l)
        for i in range(1, 12):
            m = m * (k * k * k - 16 * k) // (i * i * i)
            l += 545140134
            x *= -262537412640768000
            total += Decimal(m * l) / x
            k += 12
        pi = Decimal(426880) * Decimal(10005).sqrt() / total
        p1 = round32(pi / 32)
        p2 = round32(pi / 32 - Decimal(p1))
        p3 = float(pi / 32 - Decimal(p1) - Decimal(p2))
        quarter = [(Decimal(0), Decimal(1))]
        quarter.extend((series(pi * i / 32), series(pi * i / 32, True)) for i in range(1, 16))
        quarter.append((Decimal(1), Decimal(0)))
        rows = []
        for i in range(64):
            q, j = divmod(i, 16)
            s, c = quarter[j]
            s, c = ((s, c), (c, -s), (-s, -c), (-c, s))[q]
            # The compensated sine residual is rounded to binary32, then widened.
            hi = float(s)
            lo = struct.unpack(">f", struct.pack(">f", float(s - Decimal(hi))))[0]
            if c:
                sign = -1 if c < 0 else 1
                sigma = min((Decimal(2) ** exponent for exponent in range(-4, 1)), key=lambda v: abs(abs(c) - v)) * sign
            else:
                sigma = Decimal(0)
            vals = (float(c - sigma), hi, float(lo), float(sigma))
            rows.append([bits(v if v else 0.0) for v in vals])
        coefficients = [[bits(float(Decimal(a) / b)), bits(float(Decimal(c) / d))]
                        for a, b, c, d in [(-1, 6, -1, 2), (1, 120, 1, 24), (-1, 5040, -1, 720), (1, 362880, 1, 40320)]]
        return {"construction": "120-digit Chudnovsky pi; Taylor sin/cos; binary32 sine residual; 32-significant-bit pi splits",
                "pi32_inverse": bits(float(32 / pi)), "pi32_split": [bits(p1), bits(p2), bits(p3)],
                "coefficients": coefficients, "table": rows}


def check_source(table):
    source = ROOT / "target/full-parity-20261002/misc-math-source/stubGenerator_x86_64_constants.cpp"
    text = source.read_text(encoding="utf-8")

    def words(name):
        body = re.search(r"\b_" + re.escape(name) + r"\[\]\s*=\s*\{([^}]+)\}", text).group(1)
        values = [int(word, 16) for word in re.findall(r"0x([0-9a-f]+)UL", body)]
        return [lo | (hi << 32) for lo, hi in zip(values[::2], values[1::2])]

    expected = words("Ctable")
    rows = [expected[i:i + 4] for i in range(0, len(expected), 4)]
    assert table["table"] == rows, [(i, a, b) for i, (a, b) in enumerate(zip(table["table"], rows)) if a != b]
    assert table["pi32_inverse"] == words("PI32INV")[0]
    assert table["pi32_split"] == [words(name)[0] for name in ("P_1", "P_2", "P_3")]
    assert table["coefficients"] == [words(name) for name in ("SC_1", "SC_2", "SC_3", "SC_4")]
    print("All independently derived numeric constants equal the OpenJDK 25 table.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check-source", action="store_true")
    args = parser.parse_args()
    data = make_table()
    if args.check_source:
        check_source(data)
    data["generator_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    if args.output:
        if args.output.exists() and json.loads(args.output.read_text(encoding="utf-8")) != data:
            raise ValueError("Refusing to replace differing mathematical input data")
        args.output.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
        print(args.output)


if __name__ == "__main__":
    main()
