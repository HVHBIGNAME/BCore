"""Capture real *terrain* `map_chunk` payloads from a vanilla 26.2 server.

The existing captures come from a superflat world, where every section is a
single-value palette with no fluids. Realistic terrain exercises paths flat
chunks never touch, and this script records them so the encoder can be checked
against ground truth:

* multi-entry (indirect) block palettes, and the direct/global palette,
* a non-zero `fluidCount` (water sections),
* heightmaps whose three kinds actually differ (leaves are in `WORLD_SURFACE`
  and `MOTION_BLOCKING` but not `MOTION_BLOCKING_NO_LEAVES`),
* varying sky-light section masks.

It also *decodes* each chunk locally and prints a summary so the wire semantics
(especially `fluidCount`) can be read off directly rather than guessed.

usage: python scripts/capture_terrain.py [port]
"""

from __future__ import annotations

import socket
import struct
import sys
import time

HOST = "127.0.0.1"
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 25571
OUT = "crates/bcore-protocol/data/vanilla_terrain_chunks.bin"

CB_CHUNK_BATCH_FINISHED = 0x0B
CB_KICK = 0x20
CB_KEEP_ALIVE = 0x2C
CB_MAP_CHUNK = 0x2D
CB_POSITION = 0x48
SB_TELEPORT_CONFIRM = 0x00
SB_CHUNK_BATCH_RECEIVED = 0x0B
SB_KEEP_ALIVE = 0x1C
SB_POSITION = 0x1E
SB_PLAYER_LOADED = 0x2C

SECTION_COUNT = 24
SECTION_VOLUME = 4096
SECTION_BIOMES = 64


def wv(v: int) -> bytes:
    out = b""
    v &= 0xFFFFFFFF
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            b |= 0x80
        out += bytes([b])
        if not v:
            return out


def ws(s: str) -> bytes:
    b = s.encode("utf-8")
    return wv(len(b)) + b


def wp(pid: int, data: bytes = b"") -> bytes:
    body = wv(pid) + data
    return wv(len(body)) + body


def read_n(sock: socket.socket, n: int) -> bytes:
    d = b""
    while len(d) < n:
        c = sock.recv(n - len(d))
        if not c:
            raise EOFError("closed")
        d += c
    return d


def rv(sock: socket.socket) -> int:
    r = 0
    for i in range(5):
        b = read_n(sock, 1)[0]
        r |= (b & 0x7F) << (7 * i)
        if not (b & 0x80):
            return r
    raise ValueError("varint")


def parse_varint(b: bytes, at: int = 0) -> tuple[int, int]:
    r = 0
    for j in range(5):
        x = b[at + j]
        r |= (x & 0x7F) << (7 * j)
        if not (x & 0x80):
            return r, j + 1
    raise ValueError("varint")


def rp(sock: socket.socket) -> tuple[int, bytes]:
    n = rv(sock)
    data = read_n(sock, n)
    pid, used = parse_varint(data)
    return pid, data[used:]


class Reader:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.at = 0

    def varint(self) -> int:
        v, used = parse_varint(self.data, self.at)
        self.at += used
        return v

    def i16(self) -> int:
        v = struct.unpack_from(">h", self.data, self.at)[0]
        self.at += 2
        return v

    def u8(self) -> int:
        v = self.data[self.at]
        self.at += 1
        return v

    def longs(self, n: int) -> list[int]:
        out = list(struct.unpack_from(f">{n}q", self.data, self.at))
        self.at += n * 8
        return out


def unpack(bits: int, longs: list[int], entries: int) -> list[int]:
    if bits == 0:
        return [0] * entries
    per_long = 64 // bits
    mask = (1 << bits) - 1
    out = []
    for i in range(entries):
        word = longs[i // per_long] & 0xFFFFFFFFFFFFFFFF
        out.append((word >> ((i % per_long) * bits)) & mask)
    return out


def read_container(r: Reader, entries: int) -> tuple[int, list[int], list[int]]:
    """Return (bits, palette, values). Data array is NOT length-prefixed."""
    bits = r.u8()
    if bits == 0:
        single = r.varint()
        return bits, [single], [single] * entries
    palette = [r.varint() for _ in range(r.varint())]
    per_long = 64 // bits
    n_longs = (entries + per_long - 1) // per_long
    longs = r.longs(n_longs)
    indices = unpack(bits, longs, entries)
    return bits, palette, [palette[i] for i in indices]


def decode_chunk(payload: bytes) -> dict:
    r = Reader(payload)
    x = struct.unpack_from(">i", r.data, r.at)[0]
    r.at += 4
    z = struct.unpack_from(">i", r.data, r.at)[0]
    r.at += 4

    heightmaps = {}
    for _ in range(r.varint()):
        kind = r.varint()
        n = r.varint()
        heightmaps[kind] = r.longs(n)

    data_len = r.varint()
    end = r.at + data_len
    sections = []
    while r.at < end:
        block_count = r.i16()
        fluid_count = r.i16()
        bits, palette, states = read_container(r, SECTION_VOLUME)
        bbits, bpalette, biomes = read_container(r, SECTION_BIOMES)
        sections.append(
            {
                "block_count": block_count,
                "fluid_count": fluid_count,
                "bits": bits,
                "palette": palette,
                "states": states,
                "biome_bits": bbits,
                "biome_palette": bpalette,
                "biomes": biomes,
            }
        )
    assert r.at == end, f"section overrun: {r.at} != {end}"

    n_block_entities = r.varint()
    return {
        "x": x,
        "z": z,
        "heightmaps": heightmaps,
        "sections": sections,
        "block_entities": n_block_entities,
        "size": len(payload),
    }


def main() -> None:
    s = socket.create_connection((HOST, PORT), timeout=20)
    s.settimeout(20)
    s.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack(">H", PORT) + wv(2)))
    s.sendall(wp(0x00, ws("BCoreTerrain") + bytes(range(16))))

    pid, data = rp(s)
    assert pid == 0x02, f"login: got {pid:#x}"
    print("[login] success")
    s.sendall(wp(0x03))

    while True:
        pid, data = rp(s)
        if pid == 0x0E:
            s.sendall(wp(0x07, wv(0)))
        elif pid == 0x00:
            s.sendall(wp(0x01, data + wv(0)))
        elif pid == 0x02:
            print("[config] disconnect:", data.decode("utf-8", "replace"))
            sys.exit(1)
        elif pid == 0x03:
            break
    print("[config] done")
    s.sendall(wp(0x03))

    chunks: dict[tuple[int, int], bytes] = {}
    spawn = None
    kick = None
    s.settimeout(5)

    deadline = time.time() + 25
    while time.time() < deadline:
        try:
            pid, data = rp(s)
        except (socket.timeout, EOFError):
            break
        if pid == CB_MAP_CHUNK:
            cx, cz = struct.unpack(">ii", data[:8])
            chunks.setdefault((cx, cz), data)
        elif pid == CB_POSITION:
            tid, used = parse_varint(data)
            spawn = struct.unpack(">ddd", data[used : used + 24])
            s.sendall(wp(SB_TELEPORT_CONFIRM, wv(tid)))
            s.sendall(wp(SB_PLAYER_LOADED))
        elif pid == CB_KEEP_ALIVE:
            s.sendall(wp(SB_KEEP_ALIVE, data))
        elif pid == CB_CHUNK_BATCH_FINISHED:
            s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack(">f", 16.0)))
            if spawn is not None and len(chunks) >= 25:
                break
        elif pid == CB_KICK:
            kick = "".join(chr(c) if 32 <= c < 127 else "." for c in data)
            break

    print(f"[join] spawn={spawn} chunks={len(chunks)} kick={kick}")
    if not chunks:
        print("FAIL: no chunks received")
        sys.exit(1)

    # Walk to collect more varied terrain (hopefully crossing water).
    if spawn is not None:
        x0, y0, z0 = spawn
        walk_deadline = time.time() + 60  # hard budget: never hang the harness
        for i in range(1, 40):
            if time.time() > walk_deadline:
                print("[walk] time budget reached")
                break
            x = x0 + 6.0 * i
            z = z0 + 6.0 * (i // 3)
            try:
                s.sendall(wp(SB_POSITION, struct.pack(">ddd", x, y0, z) + bytes([0x01])))
            except OSError:
                break
            time.sleep(0.05)
            s.settimeout(0.25)
            drained = 0
            while drained < 400:
                drained += 1
                try:
                    pid, data = rp(s)
                except (socket.timeout, EOFError, OSError):
                    break
                if pid == CB_MAP_CHUNK:
                    cx, cz = struct.unpack(">ii", data[:8])
                    chunks.setdefault((cx, cz), data)
                elif pid == CB_KEEP_ALIVE:
                    s.sendall(wp(SB_KEEP_ALIVE, data))
                elif pid == CB_CHUNK_BATCH_FINISHED:
                    s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack(">f", 16.0)))
                elif pid == CB_KICK:
                    kick = "".join(chr(c) if 32 <= c < 127 else "." for c in data)
                    break
            if kick:
                break

    print(f"[walk] collected {len(chunks)} chunks; kick={kick}")

    # --- decode and report what the wire format actually says ---------------
    water_ids = set()
    print()
    print("=== DECODED SUMMARY ===")
    interesting = []
    for (cx, cz), payload in sorted(chunks.items()):
        try:
            info = decode_chunk(payload)
        except Exception as exc:  # noqa: BLE001 - diagnostic script
            print(f"  ({cx},{cz}) decode failed: {exc}")
            continue
        fluids = sum(sec["fluid_count"] for sec in info["sections"])
        max_bits = max(sec["bits"] for sec in info["sections"])
        palettes = max(len(sec["palette"]) for sec in info["sections"])
        biome_bits = max(sec["biome_bits"] for sec in info["sections"])
        hm_kinds = sorted(info["heightmaps"])
        if fluids:
            interesting.append((cx, cz, fluids))
        print(
            f"  ({cx:>4},{cz:>4}) size={info['size']:>6} fluidTotal={fluids:>5} "
            f"maxBits={max_bits} maxPalette={palettes} biomeBits={biome_bits} "
            f"hmKinds={hm_kinds} blockEntities={info['block_entities']}"
        )

    # Find the exact fluid semantics from a section with water.
    print()
    print("=== fluidCount SEMANTICS ===")
    for (cx, cz), payload in sorted(chunks.items()):
        try:
            info = decode_chunk(payload)
        except Exception:  # noqa: BLE001
            continue
        for i, sec in enumerate(info["sections"]):
            if sec["fluid_count"] == 0:
                continue
            # Which palette entries look like fluids? Count each palette id.
            counts: dict[int, int] = {}
            for state in sec["states"]:
                counts[state] = counts.get(state, 0) + 1
            non_air = sum(v for k, v in counts.items() if k != 0)
            print(f"  chunk ({cx},{cz}) section {i} (y {-64 + i * 16}..{-64 + i * 16 + 15}):")
            print(f"    blockCount={sec['block_count']} fluidCount={sec['fluid_count']}")
            print(f"    non-air blocks counted = {non_air}")
            top = sorted(counts.items(), key=lambda kv: -kv[1])[:8]
            print(f"    most common states: {top}")
            # A state whose count equals fluidCount is very likely the fluid.
            for state, count in counts.items():
                if count == sec["fluid_count"] and state != 0:
                    water_ids.add(state)
                    print(f"    >>> state {state} has exactly fluidCount ({count}) blocks")
            break
        if water_ids:
            break

    if water_ids:
        print(f"\n  candidate water/fluid state ids: {sorted(water_ids)}")
        print("  (minecraft:water level=0 is 86 per the datagen block report)")

    items = sorted(chunks.items())
    with open(OUT, "wb") as f:
        f.write(struct.pack(">I", len(items)))
        for _, payload in items:
            f.write(struct.pack(">i", CB_MAP_CHUNK))
            f.write(struct.pack(">I", len(payload)))
            f.write(payload)
    print(f"\n[out] wrote {len(items)} terrain chunks to {OUT}")
    print(f"[out] chunks containing fluids: {len(interesting)}")

    try:
        s.close()
    except OSError:
        pass


if __name__ == "__main__":
    main()
