"""Bounded, complete wire decoders for the 26.2 socket probe's control packets."""
from __future__ import annotations

from collections import Counter
import hashlib
import math
import re
import struct
import uuid

MAX_FRAME = (1 << 21) - 1
MAX_COOKIE = 5120
IDENTIFIER = re.compile(r"[a-z0-9_.-]+:[a-z0-9/._-]+\Z")


class ProtocolError(ValueError):
    """A packet violates the selected wire contract."""


def varint(value: int) -> bytes:
    if not -(1 << 31) <= value < (1 << 31):
        raise ProtocolError("VarInt outside i32")
    value &= 0xffffffff
    encoded = bytearray()
    while True:
        byte = value & 0x7f
        value >>= 7
        encoded.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(encoded)


def string(value: str, limit: int = 32767) -> bytes:
    if len(value.encode("utf-16-be")) // 2 > limit:
        raise ProtocolError("string exceeds UTF-16 limit")
    encoded = value.encode("utf-8")
    return varint(len(encoded)) + encoded


def frame(packet_id: int, payload: bytes = b"") -> bytes:
    if packet_id < 0:
        raise ProtocolError("negative packet ID")
    body = varint(packet_id) + payload
    if not 0 < len(body) <= MAX_FRAME:
        raise ProtocolError("frame exceeds Minecraft's 21-bit length limit")
    return varint(len(body)) + body


def frame_prefix(buffer: bytes | bytearray) -> tuple[int, int] | None:
    length = 0
    for index in range(3):
        if index >= len(buffer):
            return None
        byte = buffer[index]
        length |= (byte & 0x7f) << (7 * index)
        if byte & 0x80:
            continue
        prefix = index + 1
        if not 0 < length <= MAX_FRAME or buffer[:prefix] != varint(length):
            raise ProtocolError("invalid or noncanonical frame length")
        return prefix + length, prefix
    raise ProtocolError("frame length exceeds three bytes")


def split_frame(buffer: bytes | bytearray) -> tuple[int, int, bytes] | None:
    header = frame_prefix(buffer)
    if header is None or len(buffer) < header[0]:
        return None
    size, prefix = header
    reader = Reader(bytes(buffer[prefix:size]))
    packet_id = reader.varint()
    if packet_id < 0:
        raise ProtocolError("negative packet ID")
    return size, packet_id, reader.take(reader.remaining)


class Reader:
    def __init__(self, data: bytes):
        self.data = data
        self.at = 0
        self.nbt_nodes = 0

    @property
    def remaining(self) -> int:
        return len(self.data) - self.at

    def take(self, size: int) -> bytes:
        if not isinstance(size, int) or not 0 <= size <= self.remaining:
            raise ProtocolError(f"truncated payload at {self.at}: need {size}, have {self.remaining}")
        result = self.data[self.at:self.at + size]
        self.at += size
        return result

    def number(self, fmt: str):
        value = struct.unpack(">" + fmt, self.take(struct.calcsize(">" + fmt)))[0]
        if isinstance(value, float) and not math.isfinite(value):
            raise ProtocolError(f"non-finite {fmt} at {self.at}")
        return value

    def byte(self) -> int:
        return self.number("B")

    def boolean(self) -> bool:
        value = self.byte()
        if value not in (0, 1):
            raise ProtocolError(f"noncanonical Boolean {value}")
        return bool(value)

    def variable(self, bits: int) -> int:
        start, value = self.at, 0
        for index in range((bits + 6) // 7):
            byte = self.byte()
            value |= (byte & 0x7f) << (index * 7)
            if byte & 0x80:
                continue
            if value >= 1 << bits:
                raise ProtocolError(f"VarInt{bits} overflow")
            signed = value - (1 << bits) if value & (1 << (bits - 1)) else value
            if bits == 32 and self.data[start:self.at] != varint(signed):
                raise ProtocolError("noncanonical VarInt")
            return signed
        raise ProtocolError(f"VarInt{bits} is too long")

    def varint(self) -> int:
        return self.variable(32)

    def varlong(self) -> int:
        return self.variable(64)

    def count(self, limit: int = 65536) -> int:
        value = self.varint()
        if not 0 <= value <= limit:
            raise ProtocolError(f"count {value} outside 0..{limit}")
        return value

    def string(self, limit: int = 32767) -> str:
        size = self.count(limit * 3)
        try:
            value = self.take(size).decode("utf-8", "strict")
        except UnicodeDecodeError as error:
            raise ProtocolError("invalid UTF-8 string") from error
        if len(value.encode("utf-16-be")) // 2 > limit:
            raise ProtocolError("string exceeds UTF-16 limit")
        return value

    def identifier(self) -> str:
        value = self.string()
        if not IDENTIFIER.fullmatch(value):
            raise ProtocolError(f"invalid resource identifier {value!r}")
        return value

    def uuid(self) -> str:
        return str(uuid.UUID(bytes=self.take(16)))

    def finish(self) -> None:
        if self.remaining:
            raise ProtocolError(f"trailing {self.remaining} payload bytes at {self.at}")

    def nbt(self, depth: int = 0, tag: int | None = None) -> dict:
        if depth > 64 or self.nbt_nodes >= 1_000_000:
            raise ProtocolError("NBT structural budget exceeded")
        self.nbt_nodes += 1
        tag = self.byte() if tag is None else tag
        scalars = {1: ("Byte", "b"), 2: ("Short", "h"), 3: ("Int", "i"), 4: ("Long", "q")}
        if tag in scalars:
            name, fmt = scalars[tag]
            return {name: self.number(fmt)}
        if tag in (5, 6):
            raw = self.take(4 if tag == 5 else 8)
            value = struct.unpack(">f" if tag == 5 else ">d", raw)[0]
            return {"Float" if tag == 5 else "Double": {
                "bits": raw.hex(), "value": value if math.isfinite(value) else None}}
        if tag == 8:
            return {"String": modified_utf8(self.take(self.number("H")))}
        if tag == 9:
            element, count = self.byte(), self.number("i")
            if not 0 <= count <= 1_000_000 or not 0 <= element <= 12 or element == 0 and count:
                raise ProtocolError("invalid NBT list")
            return {"List": {"element_type": element, "values": [
                self.nbt(depth + 1, element) for _ in range(count)]}}
        if tag == 10:
            entries = {}
            while True:
                child = self.byte()
                if child == 0:
                    return {"Compound": entries}
                name = modified_utf8(self.take(self.number("H")))
                if name in entries:
                    raise ProtocolError(f"duplicate NBT key {name!r}")
                entries[name] = self.nbt(depth + 1, child)
        if tag in (7, 11, 12):
            name, fmt, width = {7: ("ByteArray", "b", 1), 11: ("IntArray", "i", 4),
                                12: ("LongArray", "q", 8)}[tag]
            count = self.number("i")
            if not 0 <= count <= self.remaining // width:
                raise ProtocolError("invalid NBT array length")
            return {name: list(struct.unpack(">" + fmt * count, self.take(width * count)))}
        raise ProtocolError(f"invalid anonymous NBT type {tag}")


def modified_utf8(raw: bytes) -> str:
    units, index = bytearray(), 0
    while index < len(raw):
        first = raw[index]
        index += 1
        if 0 < first < 0x80:
            value = first
        elif 0xc0 <= first < 0xe0 and index < len(raw):
            second = raw[index]
            index += 1
            value = ((first & 31) << 6) | (second & 63)
            if second & 0xc0 != 0x80 or value < 0x80 and (first, second) != (0xc0, 0x80):
                raise ProtocolError("invalid modified UTF-8 sequence")
        elif 0xe0 <= first < 0xf0 and index + 1 < len(raw):
            second, third = raw[index:index + 2]
            index += 2
            value = ((first & 15) << 12) | ((second & 63) << 6) | (third & 63)
            if second & 0xc0 != 0x80 or third & 0xc0 != 0x80 or value < 0x800:
                raise ProtocolError("invalid modified UTF-8 sequence")
        else:
            raise ProtocolError("invalid modified UTF-8 leading byte")
        units.extend(struct.pack(">H", value))
    return units.decode("utf-16-be", "surrogatepass")


def properties(reader: Reader) -> list[dict]:
    result = []
    for _ in range(reader.count(1024)):
        name, value = reader.string(), reader.string()
        signature = reader.string() if reader.boolean() else None
        result.append({"name": name, "value": value, "signature": signature})
    return result


def login_finished(payload: bytes) -> dict:
    reader = Reader(payload)
    result = {"profile_uuid": reader.uuid(), "username": reader.string(16),
              "properties": properties(reader), "sessionUUID": reader.uuid()}
    reader.finish()
    return result


def known_packs(payload: bytes) -> list[dict]:
    reader = Reader(payload)
    result = [{"namespace": reader.string(), "id": reader.string(), "version": reader.string()}
              for _ in range(reader.count(64))]
    reader.finish()
    return result


def registry_data(payload: bytes) -> dict:
    reader = Reader(payload)
    name, entries, seen = reader.identifier(), [], set()
    for index in range(reader.count()):
        key = reader.identifier()
        if key in seen:
            raise ProtocolError(f"duplicate registry key {name}/{key}")
        seen.add(key)
        if not reader.boolean():
            raise ProtocolError(f"registry {name}/{key} omits data after empty known-packs response")
        entries.append({"id": index, "key": key, "data": reader.nbt()})
    reader.finish()
    return {"registry": name, "entries": entries, "full_data": True}


def enabled_features(payload: bytes) -> list[str]:
    reader = Reader(payload)
    result = [reader.identifier() for _ in range(reader.count(256))]
    reader.finish()
    if len(set(result)) != len(result):
        raise ProtocolError("duplicate feature flag")
    return result


def tags(payload: bytes) -> dict:
    reader, result = Reader(payload), {}
    for _ in range(reader.count(1024)):
        registry = reader.identifier()
        if registry in result:
            raise ProtocolError("duplicate tag registry")
        rows = {}
        for _ in range(reader.count()):
            name = reader.identifier()
            if name in rows:
                raise ProtocolError("duplicate tag key")
            rows[name] = [reader.count(1 << 20) for _ in range(reader.count(1 << 20))]
        result[registry] = rows
    reader.finish()
    return result


def cookie_key(payload: bytes) -> str:
    reader = Reader(payload)
    key = reader.identifier()
    reader.finish()
    return key


def store_cookie(payload: bytes) -> tuple[str, bytes]:
    reader = Reader(payload)
    key = reader.identifier()
    value = reader.take(reader.count(MAX_COOKIE))
    reader.finish()
    return key, value


def cookie_response(key: str, value: bytes | None) -> bytes:
    if value is None:
        return string(key) + b"\x00"
    if len(value) > MAX_COOKIE:
        raise ProtocolError("cookie exceeds 5120 bytes")
    return string(key) + b"\x01" + varint(len(value)) + value


def global_position(reader: Reader) -> dict:
    dimension, packed = reader.identifier(), reader.number("Q")
    def signed(value: int, bits: int) -> int:
        return value - (1 << bits) if value & (1 << (bits - 1)) else value
    return {"dimension": dimension, "x": signed(packed >> 38, 26),
            "z": signed((packed >> 12) & 0x3ffffff, 26), "y": signed(packed & 0xfff, 12)}


def play_login(payload: bytes) -> dict:
    reader = Reader(payload)
    result = {"entity_id": reader.number("i"), "hardcore": reader.boolean()}
    result["world_names"] = [reader.identifier() for _ in range(reader.count(64))]
    for key in ("max_players", "view_distance", "simulation_distance"):
        result[key] = reader.count(1024)
    for key in ("reduced_debug_info", "show_death_screen", "limited_crafting"):
        result[key] = reader.boolean()
    # DimensionType.STREAM_CODEC uses holderRegistry: the raw registry ID, including zero.
    result.update(dimension_type=reader.count(), dimension=reader.identifier(), hashed_seed=reader.number("q"),
                  gamemode=reader.byte(), previous_gamemode=reader.number("b"),
                  debug=reader.boolean(), flat=reader.boolean())
    result["last_death"] = global_position(reader) if reader.boolean() else None
    result.update(portal_cooldown=reader.count(), sea_level=reader.varint(),
                  onlineMode=reader.boolean(), enforcesSecureChat=reader.boolean())
    reader.finish()
    if result["gamemode"] not in range(4) or result["previous_gamemode"] not in (-1, 0, 1, 2, 3):
        raise ProtocolError("invalid play gamemode")
    return result


def position(payload: bytes) -> dict:
    reader = Reader(payload)
    teleport_id = reader.count(0x7fffffff)
    confirm = payload[:reader.at]
    result = {"teleport_id": teleport_id, "confirm_hex": confirm.hex()}
    for name in ("x", "y", "z", "dx", "dy", "dz"):
        result[name] = reader.number("d")
    result.update(yaw=reader.number("f"), pitch=reader.number("f"), flags=reader.number("I"))
    reader.finish()
    if result["flags"] & ~0x1ff:
        raise ProtocolError("unknown position-relative flags")
    return result


def player_info(payload: bytes) -> dict:
    reader = Reader(payload)
    actions, rows = reader.byte(), []
    for _ in range(reader.count(1024)):
        row = {"uuid": reader.uuid()}
        if actions & 1:
            row.update(name=reader.string(16), properties=properties(reader))
        if actions & 2:
            if reader.boolean():
                row["chat_session"] = {"uuid": reader.uuid(), "expires": reader.number("q"),
                                       "key_hex": reader.take(reader.count()).hex(),
                                       "signature_hex": reader.take(reader.count()).hex()}
            else:
                row["chat_session"] = None
        if actions & 4:
            row["gamemode"] = reader.count(3)
        if actions & 8:
            row["listed"] = reader.boolean()
        if actions & 16:
            row["latency"] = reader.count(0x7fffffff)
        if actions & 32:
            row["display_name"] = reader.nbt() if reader.boolean() else None
        # Native 26.2's enum order is LIST_ORDER (bit 6), then HAT (bit 7).
        if actions & 64:
            row["list_order"] = reader.varint()
        if actions & 128:
            row["show_hat"] = reader.boolean()
        rows.append(row)
    reader.finish()
    return {"actions": actions, "entries": rows}


def world_time(payload: bytes) -> dict:
    reader = Reader(payload)
    result = {"game_time": reader.number("q"), "clocks": []}
    for _ in range(reader.count(1024)):
        result["clocks"].append({"id": reader.count(), "ticks": reader.varlong(),
                                 "partial_tick": reader.number("f"), "rate": reader.number("f")})
    reader.finish()
    return result


def chat(payload: bytes, kind: str) -> dict:
    reader = Reader(payload)
    if kind == "system_chat":
        result = {"content": reader.nbt(), "action_bar": reader.boolean()}
    else:
        result = {}
        if kind == "player_chat":
            result.update(global_index=reader.count(0x7fffffff), sender_uuid=reader.uuid(),
                          index=reader.count(0x7fffffff))
            result["signature_hex"] = reader.take(256).hex() if reader.boolean() else None
            result.update(message=reader.string(256), timestamp=reader.number("q"), salt=reader.number("q"))
            previous = []
            for _ in range(reader.count(20)):
                index = reader.count(0x7fffffff)
                previous.append({"id": index, "signature_hex": reader.take(256).hex() if index == 0 else None})
            result["previous_messages"] = previous
            result["unsigned_content"] = reader.nbt() if reader.boolean() else None
            result["filter_type"] = reader.count(2)
            if result["filter_type"] == 2:
                result["filter_mask"] = [reader.number("Q") for _ in range(reader.count(64))]
        elif kind == "disguised_chat":
            result["content"] = reader.nbt()
        else:
            raise ProtocolError(f"unknown chat kind {kind}")
        result["chat_type_holder"] = reader.count()
        if result["chat_type_holder"] == 0:
            raise ProtocolError("inline chat type requires the native schema verifier")
        result["network_name"] = reader.nbt()
        result["network_target"] = reader.nbt() if reader.boolean() else None
    reader.finish()
    return result


def text_values(value) -> list[str]:
    if isinstance(value, str):
        return [value]
    if isinstance(value, dict):
        return [text for child in value.values() for text in text_values(child)]
    if isinstance(value, list):
        return [text for child in value for text in text_values(child)]
    return []


def palette_values(reader: Reader, entries: int, registry_count: int, indirect_limit: int, floor: int):
    bits = reader.byte()
    if bits == 0:
        value = reader.count(registry_count - 1)
        for _ in range(entries):
            yield value
        return
    if bits <= indirect_limit:
        if bits < floor:
            raise ProtocolError("palette bits below the native floor")
        values = [reader.count(registry_count - 1) for _ in range(reader.count(1 << bits))]
        if not values or len(set(values)) != len(values):
            raise ProtocolError("empty or duplicate local palette")
    else:
        if bits != (registry_count - 1).bit_length():
            raise ProtocolError("global palette width differs from the authenticated registry")
        values = None
    per_long, mask = 64 // bits, (1 << bits) - 1
    for base in range(0, entries, per_long):
        packed = reader.number("Q")
        for offset in range(min(per_long, entries - base)):
            index = (packed >> (offset * bits)) & mask
            if values is not None:
                if index >= len(values):
                    raise ProtocolError("local palette index outside palette")
                index = values[index]
            elif index >= registry_count:
                raise ProtocolError("global palette ID outside registry")
            yield index


def palette(reader: Reader, entries: int, registry_count: int, indirect_limit: int, floor: int) -> Counter:
    return Counter(palette_values(reader, entries, registry_count, indirect_limit, floor))


def chunk(payload: bytes, *, height: int, block_states: int, biomes: int,
          watch: tuple[int, int, int] | None = None) -> dict:
    reader = Reader(payload)
    result = {"x": reader.number("i"), "z": reader.number("i"), "heightmaps": [], "sections": []}
    if not 16 <= height <= 4096 or height % 16:
        raise ProtocolError("unsupported dimension height")
    if watch is not None and (len(watch) != 3 or any(type(value) is not int for value in watch)
                              or not 0 <= watch[0] < 16 or not 0 <= watch[1] < height or not 0 <= watch[2] < 16):
        raise ProtocolError("watched block must use local X/Z and Y relative to the dimension minimum")
    bits = height.bit_length()
    for _ in range(reader.count(6)):
        kind, count = reader.count(5), reader.count(256)
        if count != math.ceil(256 / (64 // bits)):
            raise ProtocolError("heightmap long count differs from dimension height")
        values = [reader.number("Q") for _ in range(count)]
        for index in range(256):
            value = (values[index // (64 // bits)] >> ((index % (64 // bits)) * bits)) & ((1 << bits) - 1)
            if value > height:
                raise ProtocolError("heightmap value exceeds dimension height")
        result["heightmaps"].append({"kind": kind, "longs": [str(value) for value in values]})
    sections = Reader(reader.take(reader.count(MAX_FRAME)))
    for section in range(height // 16):
        blocks, fluids = sections.number("h"), sections.number("h")
        if not 0 <= blocks <= 4096 or not 0 <= fluids <= 4096:
            raise ProtocolError("section counts outside 0..4096")
        if watch is not None and section == watch[1] // 16:
            values = list(palette_values(sections, 4096, block_states, 8, 4))
            result["watched_block_state"] = values[(watch[1] % 16) * 256 + watch[2] * 16 + watch[0]]
            states = Counter(values)
        else:
            states = palette(sections, 4096, block_states, 8, 4)
        biome_counts = palette(sections, 64, biomes, 3, 1)
        result["sections"].append({"block_count": blocks, "fluid_count": fluids,
                                    "block_states": dict(states), "biomes": dict(biome_counts)})
    sections.finish()
    result["block_entities"] = []
    entity_positions = set()
    for _ in range(reader.count(65536)):
        packed, y, kind = reader.byte(), reader.number("h"), reader.count()
        tag = reader.byte()
        result["block_entities"].append({"x": packed >> 4, "z": packed & 15, "y": y, "type": kind,
                                          "nbt": None if tag == 0 else reader.nbt(tag=tag)})
        if (packed, y) in entity_positions or not 0 <= kind < 49:
            raise ProtocolError("duplicate block entity or type outside native 26.2 registry")
        entity_positions.add((packed, y))
    masks = [[reader.number("Q") for _ in range(reader.count(64))] for _ in range(4)]
    light_sections = height // 16 + 2
    flattened = [sum(value << (64 * index) for index, value in enumerate(mask)) for mask in masks]
    if any(value >> light_sections for value in flattened):
        raise ProtocolError("light mask exceeds dimension")
    if flattened[0] & flattened[2] or flattened[1] & flattened[3]:
        raise ProtocolError("light data and empty masks overlap")
    layers = []
    for kind in range(2):
        entries = []
        for _ in range(reader.count(light_sections)):
            size = reader.count(2048)
            if size != 2048:
                raise ProtocolError("light layer is not 2048 bytes")
            entries.append(hashlib.sha256(reader.take(size)).hexdigest())
        if len(entries) != flattened[kind].bit_count():
            raise ProtocolError("light layer count differs from mask")
        layers.append(entries)
    result["light"] = {"masks": [[str(value) for value in mask] for mask in masks], "layer_sha256": layers}
    reader.finish()
    return result
