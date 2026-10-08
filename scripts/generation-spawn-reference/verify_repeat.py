"""Verify independent native SPAWN captures, typed binary NBT, and coverage."""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[2]


class Nbt:
    def __init__(self, data):
        self.data = data
        self.offset = 0

    def number(self, form):
        value = struct.unpack_from(">" + form, self.data, self.offset)[0]
        self.offset += struct.calcsize(">" + form)
        return value

    def string(self):
        size = self.number("H")
        value = self.data[self.offset:self.offset + size].decode("utf-8")
        self.offset += size
        return value

    def payload(self, kind):
        if kind in (1, 2, 3, 4, 5, 6):
            return self.number({1: "b", 2: "h", 3: "i", 4: "q", 5: "f", 6: "d"}[kind])
        if kind in (7, 11, 12):
            length = self.number("i")
            assert length >= 0
            return [self.number({7: "b", 11: "i", 12: "q"}[kind]) for _ in range(length)]
        if kind == 8:
            return self.string()
        if kind == 9:
            child, length = self.number("B"), self.number("i")
            assert length >= 0
            return [[child, self.payload(child)] for _ in range(length)]
        if kind == 10:
            result = {}
            while child := self.number("B"):
                key = self.string()
                assert key not in result
                result[key] = [child, self.payload(child)]
            return result
        raise AssertionError(f"Unknown NBT tag {kind}")

    def root(self):
        assert self.number("B") == 10
        assert self.string() == ""
        result = [10, self.payload(10)]
        assert self.offset == len(self.data)
        return result


def canonical(tag, key="", attribute_order=True):
    kind, payload = tag
    if kind == 10:
        payload = {k: canonical(v, k, attribute_order) for k, v in sorted(payload.items())}
    elif kind == 9:
        payload = [canonical(v, attribute_order=attribute_order) for v in payload]
        if key == "attributes" and attribute_order:
            payload.sort(key=lambda v: v[1]["id"][1])
    elif kind in (5, 6):
        # Gson emits the shortest Float string, not its exact double expansion.
        payload = struct.pack(">" + ("f" if kind == 5 else "d"), payload).hex()
    return [kind, payload]


def check_entity(entity):
    assert entity["problems"] == ""
    binary = Nbt(bytes.fromhex(entity["nbt_hex"])).root()
    assert canonical(binary, attribute_order=False) == canonical(
        entity["typed_nbt"], attribute_order=False
    ), f"Binary/typed NBT mismatch: {entity['type']}"
    result = {**{k: v for k, v in entity.items() if k not in ("nbt_hex", "typed_nbt")},
              "typed_nbt": canonical(entity["typed_nbt"])}
    if "handoff" in result:
        handoff = dict(result["handoff"])
        handoff["loaded"] = check_entity(handoff["loaded"])
        packets = []
        for packet in handoff["packets"]:
            packet = dict(packet)
            if "attributes" in packet:
                # Verify the original packet before normalizing JVM identity-map
                # order. Every attribute's native codec bytes remain compared.
                expected = (varint(packet["packet_id"]) + varint(handoff["runtime_id"])
                            + varint(len(packet["attributes"]))
                            + b"".join(bytes.fromhex(a["hex"]) for a in packet["attributes"]))
                assert expected == bytes.fromhex(packet["wire_hex"])
                packet.pop("wire_hex")
                packet["attributes"] = sorted(packet["attributes"], key=lambda a: a["name"])
            packets.append(packet)
        handoff["packets"] = packets
        result["handoff"] = handoff
    return result


def varint(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return result


def plain(tag):
    kind, payload = tag
    if kind == 10:
        return {key: plain(value) for key, value in payload.items()}
    if kind == 9:
        return [plain(value) for value in payload]
    return payload


def verify(path):
    data = json.loads(path.read_text(encoding="utf-8"))
    assert data["minecraft"] == "26.1" and data["protocol"] == 775
    assert data["gameplay_ticks"] == 0
    counts, features = Counter(), Counter()
    for row in data.get("constructors", {}).values():
        row["entity"] = check_entity(row["entity"])
    for case in data["cases"]:
        for entity in case["entities"]:
            counts[entity["type"]] += 1
            nbt = plain(entity["typed_nbt"])
            if entity["type"] == "minecraft:goat":
                features["baby_goats"] += nbt["Age"] < 0
                features["screaming_goats"] += bool(nbt["IsScreamingGoat"])
                features["missing_left_horn"] += not nbt["HasLeftHorn"]
                features["missing_right_horn"] += not nbt["HasRightHorn"]
            if entity["type"] == "minecraft:frog":
                features["negative_age_frogs"] += nbt["Age"] < 0
                features["frog_" + nbt["variant"]] += 1
        case["entities"] = [check_entity(entity) for entity in case["entities"]]
    summary = {"cases": len(data["cases"]), "entities": dict(counts), "features": dict(features),
               "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    data.pop("provenance", None)
    return data, summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("first", type=Path)
    parser.add_argument("second", type=Path, nargs="?")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    paths = [args.first] + ([args.second] if args.second else [])
    assert all(path.resolve().is_relative_to(ROOT) for path in paths)
    data, summary = verify(args.first)
    result = {"first": summary}
    if args.second:
        repeated, repeat_summary = verify(args.second)
        # Only native identity-keyed AttributeMap iteration order is normalized.
        # Every RNG primitive, sensor, group event, world read and NBT tag remains.
        assert data == repeated, "Independent native SPAWN captures differ"
        result.update(second=repeat_summary, independent_repeat_equal=True)
    text = json.dumps(result, indent=2) + "\n"
    if args.report:
        assert args.report.resolve().is_relative_to(ROOT / "target")
        with args.report.open("x", encoding="utf-8") as file:
            file.write(text)
    print(text, end="")


if __name__ == "__main__":
    main()
