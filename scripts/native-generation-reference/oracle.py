"""Verify, decode and compare actual-engine history captures without altering fixtures."""

import argparse
from collections import Counter
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import struct

REPLAY_SCHEMA = 3
OBSERVATION_CONTRACT = "native_get_chunk_returned_object_v3"
STATUSES = (
    "empty", "structure_starts", "structure_references", "biomes", "noise", "surface",
    "carvers", "features", "initialize_light", "light", "spawn", "full",
)
NATIVE_INPUT_FILES = {
    "native_provenance_sha256": "provenance.json",
    "native_events_sha256": "events.jsonl",
    "native_config_sha256": "config.json",
    "native_registry_sha256": "block-states.json",
}
WG_MAPS = ("WORLD_SURFACE_WG", "OCEAN_FLOOR_WG")
RUNTIME_MAPS = ("WORLD_SURFACE", "OCEAN_FLOOR", "MOTION_BLOCKING", "MOTION_BLOCKING_NO_LEAVES")


def exact_i64(value, field):
    if not isinstance(value, str) or re.fullmatch(r"[+-]?[0-9]+", value) is None:
        raise ValueError(f"{field} must be an exact i64 string")
    number = int(value)
    if not -(1 << 63) <= number < (1 << 63):
        raise ValueError(f"{field} is outside signed i64")
    return number


def position_key(value):
    if (not isinstance(value, list) or len(value) != 2
            or any(type(v) is not int or not -(1 << 31) <= v < (1 << 31) for v in value)):
        raise ValueError("snapshot/request position must contain exactly two signed i32 integers")
    return tuple(value)


def validate_replay_plan(plan):
    if not isinstance(plan, dict):
        raise ValueError("replay plan must be an object")
    if type(plan.get("schema")) is not int or plan["schema"] != REPLAY_SCHEMA:
        raise ValueError("returned-object replay requires schema 3")
    if plan.get("observation_contract") != OBSERVATION_CONTRACT:
        raise ValueError("returned-object observation contract mismatch")
    exact_i64(plan.get("seed"), "seed")
    if exact_i64(plan.get("game_time"), "game_time") != 0:
        raise ValueError("fresh-world replay requires zero game time")
    for field in ("requests", "bootstrap_requests", "watch"):
        if not isinstance(plan.get(field), list):
            raise ValueError(f"{field} must be an explicit array")
    if plan.get("bootstrap") not in ("before_spawn", "native"):
        raise ValueError("unsupported bootstrap boundary")
    if plan["bootstrap"] == "before_spawn" and plan["bootstrap_requests"]:
        raise ValueError("before_spawn cannot invent bootstrap requests")
    if plan["bootstrap"] == "native" and plan.get("bootstrap_replayed") is not True:
        raise ValueError("native bootstrap requires its recorded synchronous requests")
    for spec in plan["bootstrap_requests"] + plan["requests"]:
        if not isinstance(spec, dict):
            raise ValueError("request must be an object")
        position_key(spec.get("pos"))
        status = spec.get("status")
        if not isinstance(status, str) or status.removeprefix("minecraft:") not in STATUSES:
            raise ValueError(f"unsupported status {status!r}")
        if type(spec.get("create", True)) is not bool:
            raise ValueError("create must be a boolean")
    for watch in plan["watch"]:
        position_key(watch)
    if len({tuple(watch) for watch in plan["watch"]}) != len(plan["watch"]):
        raise ValueError("duplicate watch position")
    if "feature_trace" in plan:
        trace = plan["feature_trace"]
        if not isinstance(trace, dict) or set(trace) != {"source", "feature"}:
            raise ValueError("feature_trace requires source and feature")
        position_key(trace["source"])
        if not isinstance(trace["feature"], str) or not trace["feature"]:
            raise ValueError("feature_trace feature must be a nonempty string")
    for field in NATIVE_INPUT_FILES:
        if not isinstance(plan.get(field), str) or re.fullmatch(r"[0-9a-f]{64}", plan[field]) is None:
            raise ValueError(f"missing/invalid native provenance: {field}")


class NbtReader:
    """Uncompressed named-root NBT. Original blob bytes remain the typed authority."""

    def __init__(self, data):
        self.stream = io.BytesIO(data)

    def take(self, size):
        if size < 0:
            raise ValueError("negative NBT length")
        value = self.stream.read(size)
        if len(value) != size:
            raise ValueError("truncated NBT")
        return value

    def number(self, fmt):
        return struct.unpack(">" + fmt, self.take(struct.calcsize(fmt)))[0]

    def string(self):
        # DataOutput uses modified UTF-8. Decode the UTF-16 surrogate pairs too.
        raw = self.take(self.number("H")).replace(b"\xc0\x80", b"\x00")
        text = raw.decode("utf-8", "surrogatepass")
        return text.encode("utf-16", "surrogatepass").decode("utf-16")

    def payload(self, kind):
        if kind in (1, 2, 3, 4, 5, 6):
            return self.number({1: "b", 2: "h", 3: "i", 4: "q", 5: "f", 6: "d"}[kind])
        if kind == 7:
            return list(self.take(self.number("i")))
        if kind == 8:
            return self.string()
        if kind == 9:
            element, length = self.number("B"), self.number("i")
            if length < 0:
                raise ValueError("negative NBT list length")
            return [self.payload(element) for _ in range(length)]
        if kind == 10:
            result = {}
            while (entry := self.number("B")) != 0:
                name = self.string()
                if name in result:
                    raise ValueError(f"duplicate compound key {name}")
                result[name] = self.payload(entry)
            return result
        if kind in (11, 12):
            length = self.number("i")
            if length < 0:
                raise ValueError("negative NBT array length")
            return [self.number("i" if kind == 11 else "q") for _ in range(length)]
        raise ValueError(f"unknown NBT type {kind}")

    def read(self):
        if self.number("B") != 10:
            raise ValueError("expected named compound root")
        self.string()
        result = self.payload(10)
        if self.stream.read(1):
            raise ValueError("trailing NBT bytes")
        return result


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for data in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(data)
    return digest.hexdigest()


def native_input_identity(directory):
    """Check original capture receipts, never rewrite them with current tool hashes."""
    identity = {field: sha(directory / name) for field, name in NATIVE_INPUT_FILES.items()}
    provenance = json.loads((directory / "provenance.json").read_text(encoding="utf-8"))
    capture = json.loads((directory / "capture.json").read_text(encoding="utf-8"))
    for expected, actual, label in (
        (provenance.get("config_sha256"), identity["native_config_sha256"], "config"),
        (capture.get("events_sha256"), identity["native_events_sha256"], "events"),
        (capture.get("provenance_sha256"), identity["native_provenance_sha256"], "provenance"),
    ):
        if expected != actual:
            raise ValueError(f"native {label} provenance digest mismatch")
    return identity


def load_events(directory):
    with (directory / "events.jsonl").open(encoding="utf-8") as stream:
        return [json.loads(line) for line in stream]


def blob(directory, name):
    if Path(name).name != name:
        raise ValueError("blob path escapes capture")
    raw = gzip.decompress((directory / "blobs" / name).read_bytes())
    if hashlib.sha256(raw).hexdigest() != name.split(".")[0]:
        raise ValueError(f"blob digest mismatch: {name}")
    return raw


def state_key(state):
    return state["Name"], tuple(sorted(state.get("Properties", {}).items()))


def unpack(palette, packed, count, minimum_bits):
    if len(palette) == 1:
        if packed:
            raise ValueError("unexpected data for a singleton palette")
        return [palette[0]] * count
    if not palette:
        raise ValueError("empty palette")
    bits = max(minimum_bits, (len(palette) - 1).bit_length())
    per_long = 64 // bits
    if len(packed) != (count + per_long - 1) // per_long:
        raise ValueError("wrong non-straddling palette storage length")
    mask = (1 << bits) - 1
    return [palette[(packed[i // per_long] >> ((i % per_long) * bits)) & mask] for i in range(count)]


def decode_snapshot(directory, snapshot, registry):
    nbt = NbtReader(blob(directory, snapshot["nbt"])).read()
    extra = json.loads(blob(directory, snapshot["extra"]))
    min_section = nbt["yPos"]
    sections = [s for s in nbt["sections"] if "block_states" in s]
    if [s["Y"] for s in sections] != list(range(min_section, min_section + len(sections))):
        raise ValueError("non-contiguous native block sections")
    states, biomes = [], []
    for section in sections:
        data = section["block_states"]
        palette = [registry[state_key(state)] for state in data["palette"]]
        states.extend(unpack(palette, data.get("data", []), 4096, 4))
        data = section["biomes"]
        biomes.extend(unpack(data["palette"], data.get("data", []), 64, 1))
    for index, value in extra.get("sample_state_ids", []):
        if states[index] != value:
            raise ValueError(f"NBT decoder differs from native LevelChunkSection.getBlockState at index {index}")
    return {"states": states, "biomes": biomes, "nbt": nbt, "extra": extra, "min_y": min_section * 16}


def references(events):
    for event in events:
        if event["event"] == "stage_snapshot":
            for chunk in event["chunks"]:
                yield chunk["snapshot"]
        elif event["event"] == "feature_snapshot":
            yield from event["chunks"]
        elif event["event"] == "empty_complete":
            yield event["snapshot"]
        elif event["event"] == "request_exit":
            yield event["snapshot"]
            yield from (s for s in event["watch"] if not s.get("absent"))


def validate_stages(events):
    if not events or events[-1]["event"] != "oracle_complete":
        raise ValueError("capture lacks a successful terminal event")
    stages = {e["seq"]: e for e in events if e["event"] == "stage_enter"}
    exits = {e["stage_id"]: e for e in events if e["event"] == "stage_exit"}
    pending = {e["stage_id"]: e for e in events[-1].get("inflight_stages", [])}
    if len(pending) != len(events[-1].get("inflight_stages", [])) or pending.keys() & exits.keys():
        raise ValueError("duplicate or completed in-flight stage declaration")
    if stages.keys() != exits.keys() | pending.keys():
        missing = [{key: stages[seq][key] for key in ("seq", "pos", "status", "request", "thread")}
                   for seq in sorted(stages.keys() - exits.keys())]
        orphaned = sorted(exits.keys() - stages.keys())
        repeated = {seq: count for seq, count in Counter(e["stage_id"] for e in events
                    if e["event"] == "stage_exit").items() if count > 1}
        raise ValueError(f"unpaired stage entry/completion: missing={missing}, orphaned={orphaned}, repeated={repeated}")
    if len(exits) != sum(e["event"] == "stage_exit" for e in events):
        raise ValueError("duplicate stage completion")
    for seq, entry in pending.items():
        if any(entry[key] != stages[seq][key] for key in ("pos", "status", "request", "thread")):
            raise ValueError("in-flight stage declaration differs from its entry")
    boundaries = Counter((e["stage_id"], e["boundary"]) for e in events if e["event"] == "stage_snapshot")
    if any(stage not in stages or boundary not in ("before", "after") for stage, boundary in boundaries):
        raise ValueError("stage snapshot lacks a matching entry or valid boundary")
    for stage in stages:
        before, after = boundaries[stage, "before"], boundaries[stage, "after"]
        if stage in pending:
            if not (0 <= after <= before <= 1):
                raise ValueError("invalid in-flight stage snapshot prefix")
        elif before != 1 or after != 1:
            raise ValueError("missing or duplicate complete stage snapshots")
    return stages, list(pending.values())


def summarize(directory, verify=False):
    events = load_events(directory)
    if events[-1]["event"] != "oracle_complete":
        raise ValueError("capture lacks a successful terminal event")
    if [e["seq"] for e in events] != list(range(1, len(events) + 1)):
        raise ValueError("non-contiguous observation sequence")
    if events[-1]["gameplay_ticks"] != 0 or events[-1]["game_time"] != 0:
        raise ValueError("generation history includes gameplay time")
    stages, pending = validate_stages(events)
    features = [e for e in stages.values() if e["status"] == "minecraft:features"]
    if [e["source_sequence"] for e in features] != list(range(1, len(features) + 1)):
        raise ValueError("invalid FEATURES source sequence")
    names = {s[field] for s in references(events) for field in ("nbt", "extra")}
    names.update(e["entity"]["nbt"] for e in events if e["event"] == "world_entity" and isinstance(e["entity"], dict))
    if verify:
        capture = json.loads((directory / "capture.json").read_text())
        if capture["events_sha256"] != sha(directory / "events.jsonl"):
            raise ValueError("events provenance digest mismatch")
        if capture["provenance_sha256"] != sha(directory / "provenance.json"):
            raise ValueError("native provenance digest mismatch")
        unchecked_blobs = list(sorted(names))
        checked = set()
        while unchecked_blobs:
            name = unchecked_blobs.pop()
            if name in checked:
                continue
            checked.add(name)
            raw = blob(directory, name)
            if ".nbt." in name:
                NbtReader(raw).read()
            else:
                extra = json.loads(raw)
                for entity in extra.get("level_entities", []):
                    unchecked_blobs.append(entity["nbt"])
                    names.add(entity["nbt"])
        registry = {state_key(s): s["id"] for s in json.loads((directory / "block-states.json").read_text())}
        # Independently recorded section.getBlockState samples catch palette index,
        # axis-order, signed-word, and section-offset mistakes in this decoder.
        seen = set()
        for snapshot in references(events):
            key = snapshot["nbt"], snapshot["extra"]
            if key not in seen:
                decode_snapshot(directory, snapshot, registry)
                seen.add(key)
    return {
        "capture": str(directory), "verified_blobs": len(names) if verify else None,
        "bootstrap": next(e for e in events if e["event"] == "bootstrap_complete"),
        "stages": dict(Counter(e["status"] for e in stages.values())),
        "completed_stages": dict(Counter(e["status"] for seq, e in stages.items() if seq not in {p["stage_id"] for p in pending})),
        "inflight_stages": pending,
        "sources": [{"pos": e["pos"], "phase": e["phase"], "request": e["request"], "seq": e["seq"]} for e in features],
        "features": dict(Counter(e["feature"] for e in events if e["event"] == "feature_enter")),
        "effects": dict(Counter(e["event"] for e in events if e["event"].startswith("world_"))),
        "write_rejections": sum(e["event"] == "world_write_guard" and not e["accepted"] for e in events),
        "gameplay_ticks": events[-1]["gameplay_ticks"],
    }


def mismatch(expected, actual, coordinates):
    absent = object()
    pairs = Counter()
    differences = []
    for i in range(max(len(expected), len(actual))):
        a = expected[i] if i < len(expected) else absent
        b = actual[i] if i < len(actual) else absent
        if a is absent or b is absent or not same_value(a, b):
            differences.append({"index": i, "pos": coordinates(i),
                                "expected_present": a is not absent, "actual_present": b is not absent,
                                "expected": None if a is absent else a,
                                "actual": None if b is absent else b})
            pairs[str(a) if a is not absent else "<missing>",
                  str(b) if b is not absent else "<missing>"] += 1
    result = {"scored": True, "matches": not differences, "mismatches": len(differences),
              "total": max(len(expected), len(actual)), "first": differences[0] if differences else None,
              "examples": differences[:16], "differences": differences,
              "pairs": [{"expected": a, "actual": b, "count": n} for (a, b), n in pairs.most_common()]}
    if len(expected) != len(actual):
        result.update(expected_length=len(expected), actual_length=len(actual))
    return result


def same_value(left, right):
    if type(left) is not type(right):
        return False
    if isinstance(left, float):
        return struct.pack(">d", left) == struct.pack(">d", right)
    if isinstance(left, (list, tuple)):
        return len(left) == len(right) and all(same_value(a, b) for a, b in zip(left, right))
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(same_value(left[key], right[key]) for key in left)
    return left == right


def locate_writes(events, position, before=None):
    active, labels, writes = {}, {}, []
    for event in events:
        if before is not None and event["seq"] >= before:
            break
        kind = event["event"]
        stage = event.get("stage_id")
        if kind == "feature_enter":
            active[stage] = event
        elif kind == "feature_exit":
            active.pop(stage, None)
        elif kind == "decoration_label":
            labels[stage] = event.get("label")
        elif kind == "world_set_block" and event["accepted"] and event["pos"] == position:
            writes.append({"write": event, "feature_enter": active.get(stage), "decoration_label": labels.get(stage)})
    return writes


def compare_serialized_light(nbt, actual):
    """Match SerializableChunkData.copyOf: omit lazy-zero, retain materialized zero.

    Native copies light layers even before isLightCorrect becomes true. Disk NBT
    cannot distinguish absent storage from lazy-zero storage; the layer oracle can.
    """
    result = {"native_is_light_on": bool(nbt.get("isLightOn", False)),
              "bcore_snapshot_available": actual is not None, "scored": False}
    if actual is None:
        return result
    if (not isinstance(actual, dict) or type(actual.get("min_section_y")) is not int
            or not isinstance(actual.get("sections"), list)
            or any(not isinstance(section, dict) for section in actual["sections"])):
        raise ValueError("malformed serialized light geometry")
    result["scored"] = True
    native = {s["Y"]: s for s in nbt["sections"]}
    expected_min = nbt["yPos"] - 1
    expected_count = sum("block_states" in s for s in nbt["sections"]) + 2
    actual_min = actual["min_section_y"]
    sections = {actual_min + i: s for i, s in enumerate(actual["sections"])}
    result["geometry_matches"] = actual_min == expected_min and len(sections) == expected_count
    result["geometry"] = nested_mismatch({"min_section_y": expected_min, "section_count": expected_count},
                                         {"min_section_y": actual_min, "section_count": len(sections)})
    result["layers"] = []
    ys = set(range(expected_min, expected_min + expected_count)) | sections.keys() | native.keys()
    for y in sorted(ys):
        section = sections.get(y, {})
        for kind, tag in (("sky", "SkyLight"), ("block", "BlockLight")):
            expected_layer = native.get(y, {}).get(tag)
            actual_layer = section.get(kind)
            if kind + "_empty" in section and type(section[kind + "_empty"]) is not bool:
                raise ValueError(f"invalid lazy-zero {kind} flag at section {y}")
            if actual_layer is not None and (not isinstance(actual_layer, list)
                    or any(type(value) is not int or not 0 <= value <= 255 for value in actual_layer)):
                raise ValueError(f"invalid {kind} light bytes at section {y}")
            if section.get(kind + "_empty", False):
                if actual_layer is None or len(actual_layer) != 2048 or any(actual_layer):
                    raise ValueError(f"invalid lazy-zero {kind} layer at section {y}")
                actual_layer = None
            layer = {"y": y, "kind": kind,
                     "expected_present": expected_layer is not None, "actual_present": actual_layer is not None,
                     "presence_matches": (expected_layer is None) == (actual_layer is None)}
            if expected_layer is not None and actual_layer is not None:
                layer["bytes"] = mismatch(expected_layer, actual_layer, lambda i: i)
            result["layers"].append(layer)
    result["presence_mismatches"] = sum(not layer["presence_matches"] for layer in result["layers"])
    result["length_mismatches"] = sum("expected_length" in layer.get("bytes", {}) for layer in result["layers"])
    result["byte_mismatches"] = sum(layer.get("bytes", {}).get("mismatches", 0) for layer in result["layers"])
    result["matches"] = result["geometry_matches"] and not any(result[key] for key in
        ("presence_mismatches", "length_mismatches", "byte_mismatches"))
    return result


def nested_mismatch(expected, actual):
    """Logical NBT/JSON comparison: preserve missing fields and every list order.

    Numeric tag widths and compound wire ordering require the typed/wire oracle.
    """
    absent = object()
    result = {"scored": True, "mismatches": 0, "examples": [], "differences": []}

    def difference(path, left, right):
        result["mismatches"] += 1
        difference = {"path": path,
            "expected_present": left is not absent, "actual_present": right is not absent,
            "expected": None if left is absent else left,
            "actual": None if right is absent else right}
        result["differences"].append(difference)
        if len(result["examples"]) < 16:
            result["examples"].append(difference)

    def visit(left, right, path):
        if isinstance(left, dict) and isinstance(right, dict):
            for key in sorted(left.keys() | right.keys()):
                visit(left.get(key, absent), right.get(key, absent), path + [key])
        elif isinstance(left, list) and isinstance(right, list):
            for index in range(max(len(left), len(right))):
                visit(left[index] if index < len(left) else absent,
                      right[index] if index < len(right) else absent, path + [index])
        elif left is absent or right is absent or not same_value(left, right):
            difference(path, left, right)

    visit(expected, actual, [])
    result["matches"] = result["mismatches"] == 0
    return result


def compare_generated_metadata(nbt, actual):
    result = {}
    structures = actual.get("structures")
    result["structures"] = ({"scored": False} if structures is None else nested_mismatch(
        nbt.get("structures"), {native: structures[key] for key, native in
                               (("starts", "starts"), ("references", "References")) if key in structures}))

    def entities_by_position(rows, native, packing=False):
        indexed = {}
        for row in rows:
            data = row if native else row["nbt"]
            position = [data[axis] for axis in ("x", "y", "z")] if native else row["pos"]
            if len(position) != 3 or any(type(v) is not int for v in position):
                raise ValueError("block entity position must contain exactly three integers")
            key = ",".join(str(v) for v in position)
            if key in indexed:
                raise ValueError(f"duplicate block entity at {key}")
            # keepPacked is LevelChunk's serialization envelope, not the block
            # entity's saved payload. Expose it separately instead of hiding it.
            indexed[key] = {key: value for key, value in data.items()
                            if (key == "keepPacked") == packing}
        return indexed

    result["block_entity_payloads"] = ({"scored": False} if "block_entities" not in actual else
        nested_mismatch(entities_by_position(nbt.get("block_entities", []), True),
                        entities_by_position(actual["block_entities"], False)))
    result["block_entity_packing"] = {"scored": False, "native_counts": dict(Counter(
        str(row["keepPacked"]) if "keepPacked" in row else "absent" for row in nbt.get("block_entities", [])))}
    boundary = actual.get("snapshot_boundary")
    if boundary is not None:
        if boundary not in ("proto_save", "level_save"):
            raise ValueError(f"unsupported snapshot boundary: {boundary}")
        if "block_entities" not in actual or "saved_ticks" not in actual:
            raise ValueError("save-boundary snapshot lacks block entities or saved ticks")
        result["block_entity_packing"].update(nested_mismatch(
            entities_by_position(nbt.get("block_entities", []), True, packing=True),
            entities_by_position(actual["block_entities"], False, packing=True)))
    result["saved_ticks"] = ({"scored": False} if boundary is None else nested_mismatch(
        {name: nbt.get(name, []) for name in ("block_ticks", "fluid_ticks")}, actual["saved_ticks"]))

    result["postprocessing"] = {"scored": False}
    if "postprocessing" in actual:
        min_section = nbt["yPos"]
        count = sum("block_states" in section for section in nbt["sections"])
        sections = [[] for _ in range(count)]
        for x, y, z in actual["postprocessing"]:
            if not (0 <= x < 16 and 0 <= z < 16 and 0 <= (y >> 4) - min_section < count):
                raise ValueError(f"postprocessing coordinate outside owned chunk: {x},{y},{z}")
            sections[(y >> 4) - min_section].append(x | ((y & 15) << 4) | (z << 8))
        result["postprocessing"] = nested_mismatch(nbt.get("PostProcessing"), sections)
    return result


def validate_snapshot(snapshot, position, origin, *, native=False):
    label = "native" if native else "BCore"
    if not isinstance(snapshot, dict):
        raise ValueError(f"missing {label} {origin} snapshot; missing is not absent")
    if position_key(snapshot.get("pos")) != position_key(position):
        raise ValueError(f"{label} {origin} snapshot position differs from its request/watch")
    if not native and snapshot.get("snapshot_origin") != origin:
        raise ValueError(f"BCore snapshot_origin must be {origin}")
    if "absent" in snapshot and type(snapshot["absent"]) is not bool:
        raise ValueError(f"{label} absent flag must be a boolean")
    if snapshot.get("absent", False):
        if any(key in snapshot for key in ("nbt", "extra", "states", "biomes", "snapshot_boundary")):
            raise ValueError(f"{label} absent snapshot also contains chunk data")
        return
    required = (("nbt", "extra", "status") if native else
                ("states", "biomes", "min_y", "raw_object_kind", "snapshot_boundary", "chunk_class",
                 "persisted_status", "serialized_status", "block_entities", "saved_ticks"))
    missing = set(required) - snapshot.keys()
    if missing:
        raise ValueError(f"{label} {origin} snapshot lacks {sorted(missing)}")
    if not native:
        if snapshot["snapshot_boundary"] not in ("proto_save", "level_save"):
            raise ValueError("unsupported snapshot boundary")
        if snapshot["raw_object_kind"] not in ("Proto", "Level", "ImposterProto"):
            raise ValueError("unsupported returned-object kind")
        if (type(snapshot["min_y"]) is not int
                or not isinstance(snapshot["states"], list) or not isinstance(snapshot["biomes"], list)):
            raise ValueError("snapshot requires integer min_y and state/biome arrays")


def validate_watches(watches, positions, *, native=False):
    if not isinstance(watches, list):
        raise ValueError("watch must be an explicit array; missing is not absent")
    keys = [position_key(w.get("pos")) if isinstance(w, dict) else position_key(None) for w in watches]
    if len(set(keys)) != len(keys):
        raise ValueError("duplicate watch snapshot position")
    if keys != [position_key(p) for p in positions]:
        raise ValueError("missing, unexpected or reordered watch position")
    for watch, position in zip(watches, positions):
        validate_snapshot(watch, position, "holder_latest", native=native)


def validate_replay_rows(rows, plan):
    validate_replay_plan(plan)
    bootstrap_count = len(plan["bootstrap_requests"])
    specs = plan["bootstrap_requests"] + plan["requests"]
    if len(rows) != len(specs):
        raise ValueError(f"request count mismatch: planned {len(specs)}, BCore {len(rows)}")
    for index, (row, spec) in enumerate(zip(rows, specs)):
        if not isinstance(row, dict) or type(row.get("schema")) is not int or row["schema"] != REPLAY_SCHEMA:
            raise ValueError("BCore history requires schema 3; rebuild legacy binaries")
        for field in ("observation_contract", "seed", "game_time", *NATIVE_INPUT_FILES):
            if row.get(field) != plan[field]:
                raise ValueError(f"BCore request {index} {field} metadata mismatch")
        phase = "bootstrap" if index < bootstrap_count else "requests"
        if (type(row.get("request")) is not int or row["request"] != index - bootstrap_count
                or row.get("phase") != phase or not nested_mismatch(spec, row.get("spec"))["matches"]):
            raise ValueError(f"BCore request {index} history/order mismatch")
        for field in ("complete", "coverage_status", "stages", "feature_sources", "snapshot", "watch"):
            if field not in row:
                raise ValueError(f"BCore request {index} lacks {field}")
        if row["complete"] is not None and type(row["complete"]) is not bool:
            raise ValueError("complete must be an explicit boolean or null")
        if row["complete"] is None:
            if row["coverage_status"] is not None or row["stages"] is not None:
                raise ValueError("absent coverage must use explicit null fields")
        else:
            if row["coverage_status"] not in ["minecraft:" + status for status in STATUSES]:
                raise ValueError("unsupported coverage status")
            stages = row["stages"]
            if (not isinstance(stages, list) or any(not isinstance(stage, dict) for stage in stages)
                    or [s.get("status") for s in stages] != ["minecraft:" + status for status in STATUSES]):
                raise ValueError("coverage must retain all ordered implementation stages")
            for stage in stages:
                if (stage.get("state") not in ("Pending", "Running", "Complete", "Partial", "Failed")
                        or type(stage.get("attempts")) is not int or stage["attempts"] < 0
                        or not isinstance(stage.get("missing"), list)
                        or any(not isinstance(reason, str) for reason in stage["missing"])):
                    raise ValueError("malformed implementation stage coverage")
        sources = row["feature_sources"]
        if not isinstance(sources, list) or any(not isinstance(source, dict) for source in sources):
            raise ValueError("feature_sources must be an explicit array")
        sequences = [s.get("sequence") for s in sources]
        if (any(type(s) is not int or s < 1 for s in sequences)
                or sequences != sorted(set(sequences))):
            raise ValueError("duplicate/unordered FEATURES source sequence")
        positions = [position_key(s.get("source")) for s in sources]
        if len(set(positions)) != len(positions):
            raise ValueError("duplicate FEATURES source position")
        if phase == "bootstrap":
            if row["snapshot"] is not None or row["watch"] != []:
                raise ValueError("bootstrap calls must not add scripted save observations")
        else:
            validate_snapshot(row["snapshot"], spec["pos"], "returned_object")
            validate_watches(row["watch"], plan["watch"])
            if row["snapshot"].get("absent", False) != (row["complete"] is None):
                raise ValueError("returned-object presence and coverage disagree")


def compare_heightmaps(decoded, actual, field, names, position):
    if field not in actual or "heightmaps_live" not in decoded["extra"]:
        return {name: {"scored": False, "reason": "heightmap observation missing"} for name in names}
    actual_maps = actual[field] if actual[field] is not None else {}
    if not isinstance(actual_maps, dict):
        raise ValueError(f"{field} must be an object or explicit null")
    native_maps = decoded["extra"]["heightmaps_live"]
    min_y = decoded["min_y"]
    palette = range(min_y, min_y + len(decoded["states"]) // 256 + 1)
    x, z = position
    results = {}
    for name in names:
        present, actual_present = name in native_maps, name in actual_maps
        result = {"scored": True, "native_present": present, "bcore_present": actual_present,
                  "presence_matches": present == actual_present, "matches": present == actual_present}
        if present and actual_present:
            result["values"] = mismatch(unpack(palette, native_maps[name], 256, 1), actual_maps[name],
                                         lambda i: [x * 16 + i % 16, z * 16 + i // 16])
            result["matches"] = result["values"]["matches"]
        results[name] = result
    return results


def unscored_snapshot_fields(native, decoded, actual):
    """Inventory unsupported fields instead of letting subset matches claim parity."""
    fields = []

    def remaining(side, path, values, consumed):
        if isinstance(values, dict):
            fields.extend({"side": side, "path": path + [key], "scored": False,
                           "reason": "no cross-engine comparison for this field"}
                          for key in sorted(values.keys() - set(consumed)))

    remaining("native", ["snapshot"], native, ("pos", "status", "nbt", "extra", "absent"))
    remaining("native", ["nbt"], decoded["nbt"],
              ("xPos", "zPos", "yPos", "sections", "Status", "structures", "block_entities",
               "block_ticks", "fluid_ticks", "PostProcessing"))
    remaining("native", ["extra"], decoded["extra"], ("heightmaps_live", "chunk_class", "min_y"))
    remaining("native", ["extra", "heightmaps_live"], decoded["extra"].get("heightmaps_live"), WG_MAPS + RUNTIME_MAPS)
    for i, section in enumerate(decoded["nbt"]["sections"]):
        remaining("native", ["nbt", "sections", i], section, ("Y", "block_states", "biomes", "SkyLight", "BlockLight"))
        for key in ("block_states", "biomes"):
            remaining("native", ["nbt", "sections", i, key], section.get(key), ("palette", "data"))
    remaining("bcore", ["snapshot"], actual,
              ("pos", "min_y", "states", "biomes", "worldgen_heightmaps", "runtime_heightmaps", "light",
               "structures", "block_entities", "saved_ticks", "postprocessing", "snapshot_origin",
               "snapshot_boundary", "chunk_class", "persisted_status", "serialized_status", "absent"))
    for field, names in (("worldgen_heightmaps", WG_MAPS), ("runtime_heightmaps", RUNTIME_MAPS)):
        remaining("bcore", ["snapshot", field], actual.get(field), names)
    remaining("bcore", ["snapshot", "structures"], actual.get("structures"), ("starts", "references"))
    for i, entity in enumerate(actual["block_entities"]):
        remaining("bcore", ["snapshot", "block_entities", i], entity, ("pos", "nbt"))
    light = actual.get("light")
    remaining("bcore", ["snapshot", "light"], light, ("min_section_y", "sections"))
    if light is not None:
        for i, section in enumerate(light["sections"]):
            remaining("bcore", ["snapshot", "light", "sections", i], section,
                      ("sky", "block", "sky_empty", "block_empty"))
    return fields


def compare_snapshot(directory, native, actual, registry, states, events, before, position, origin):
    validate_snapshot(native, position, origin, native=True)
    validate_snapshot(actual, position, origin)
    absent = native.get("absent", False), actual.get("absent", False)
    result = {"pos": position, "snapshot_origin": origin,
              "presence": nested_mismatch(not absent[0], not absent[1]),
              "native_nbt": native.get("nbt"), "native_extra": native.get("extra")}
    if any(absent):
        unscored = [{"side": side, "path": ["snapshot", key], "scored": False,
                     "reason": "no content comparison across an absent boundary"}
                    for side, snapshot in (("native", native), ("bcore", actual))
                    for key in sorted(snapshot.keys() - {"pos", "absent", "snapshot_origin"})]
        result.update(scored_matches=result["presence"]["matches"],
                      matches=False if not result["presence"]["matches"] else None if unscored else True,
                      scope="presence", unscored_fields=unscored,
                      content={"scored": False, "reason": "one or both chunks explicitly absent"})
        return result
    decoded = decode_snapshot(directory, native, registry)
    if position_key([decoded["nbt"].get("xPos"), decoded["nbt"].get("zPos")]) != position_key(position):
        raise ValueError("native NBT position differs from snapshot reference")
    if any(type(state) is not int or state not in states for state in actual["states"]):
        raise ValueError("BCore snapshot contains an unknown/non-integer state ID")
    if any(not isinstance(biome, str) for biome in actual["biomes"]):
        raise ValueError("BCore snapshot contains a non-string biome")
    x, z = position
    min_y = decoded["min_y"]
    blocks = mismatch(decoded["states"], actual["states"],
                      lambda i: [x * 16 + i % 16, min_y + i // 256, z * 16 + i // 16 % 16])
    if blocks["first"]:
        for side in ("expected", "actual"):
            if blocks["first"][side + "_present"]:
                blocks["first"][side + "_state"] = states[blocks["first"][side]]
        blocks["first"]["native_writes"] = locate_writes(events, blocks["first"]["pos"], before)
    biomes = mismatch(decoded["biomes"], actual["biomes"],
                      lambda i: [x * 4 + i % 4, min_y // 4 + i // 16, z * 4 + i // 4 % 4])
    wg = compare_heightmaps(decoded, actual, "worldgen_heightmaps", WG_MAPS, position)
    runtime = compare_heightmaps(decoded, actual, "runtime_heightmaps", RUNTIME_MAPS, position)
    light = compare_serialized_light(decoded["nbt"], actual.get("light"))
    metadata = compare_generated_metadata(decoded["nbt"], actual)
    chunk_class = decoded["extra"].get("chunk_class")
    native_boundary = {"net.minecraft.world.level.chunk.ProtoChunk": "proto_save",
                       "net.minecraft.world.level.chunk.LevelChunk": "level_save"}.get(chunk_class)
    metadata["chunk_lifecycle"] = nested_mismatch(
        {"chunk_class": chunk_class, "persisted_status": native["status"],
         "serialized_status": decoded["nbt"].get("Status")},
        {name: actual[name] for name in ("chunk_class", "persisted_status", "serialized_status")})
    metadata["save_boundary"] = ({"scored": False, "reason": "unknown native serialization class"}
        if native_boundary is None else nested_mismatch(native_boundary, actual["snapshot_boundary"]))
    geometry = nested_mismatch({"min_y": min_y, "extra_min_y": decoded["extra"].get("min_y")},
                               {"min_y": actual["min_y"], "extra_min_y": actual["min_y"]})
    comparisons = {"states": blocks, "biomes": biomes, "geometry": geometry, "serialized_light": light,
                   **{f"worldgen_heightmaps/{k}": v for k, v in wg.items()},
                   **{f"runtime_heightmaps/{k}": v for k, v in runtime.items()},
                   **{f"generated_metadata/{k}": v for k, v in metadata.items()}}
    unscored = unscored_snapshot_fields(native, decoded, actual)
    unscored.extend({"path": [name], "scored": False, "reason": value.get("reason", "observation unavailable")}
                    for name, value in comparisons.items() if not value["scored"])
    scored_matches = all(value.get("matches") is True for value in comparisons.values() if value["scored"])
    result.update(states=blocks, biomes=biomes, geometry=geometry, worldgen_heightmaps=wg,
                  runtime_heightmaps={"scored": all(v["scored"] for v in runtime.values()), "maps": runtime,
                                      "matches": (all(v["matches"] for v in runtime.values())
                                                  if all(v["scored"] for v in runtime.values()) else None)},
                  serialized_light=light, generated_metadata=metadata,
                  native_effect_counts={name: len(decoded["nbt"].get(name, [])) for name in
                                        ("block_entities", "entities", "block_ticks", "fluid_ticks")},
                  scored_matches=scored_matches, matches=False if not scored_matches else None,
                  scope="scored snapshot fields", unscored_fields=unscored)
    return result


def compare(directory, rust_path):
    identity = native_input_identity(directory)
    events = load_events(directory)
    config = json.loads((directory / "config.json").read_text(encoding="utf-8"))
    plan = plan_from_events(config, events, identity)
    actual = [json.loads(line) for line in rust_path.read_text(encoding="utf-8").splitlines()]
    validate_replay_rows(actual, plan)
    actual = actual[len(plan["bootstrap_requests"]):]
    expected = [e for e in events if e["event"] == "request_exit"]
    state_rows = json.loads((directory / "block-states.json").read_text(encoding="utf-8"))
    registry = {state_key(state): state["id"] for state in state_rows}
    states = {state["id"]: state for state in state_rows}
    if len(registry) != len(state_rows) or len(states) != len(state_rows):
        raise ValueError("duplicate native state registry entry")
    rows = []
    for native, rust in zip(expected, actual):
        validate_watches(native.get("watch"), plan["watch"], native=True)
        snapshot = compare_snapshot(directory, native.get("snapshot"), rust["snapshot"], registry, states,
                                    events, native["seq"], rust["spec"]["pos"], "returned_object")
        watches = [compare_snapshot(directory, left, right, registry, states, events, native["seq"],
                                    position, "holder_latest")
                   for position, left, right in zip(plan["watch"], native["watch"], rust["watch"])]
        native_sources = [e["pos"] for e in events if e["event"] == "stage_enter"
                          and e["status"] == "minecraft:features" and e["seq"] < native["seq"]]
        sequence = mismatch(native_sources, [s["source"] for s in rust["feature_sources"]], lambda i: i + 1)
        row = {"request": native["request"], "spec": rust["spec"], "native_event": native["seq"],
               **snapshot, "watch": watches, "source_order": sequence,
               "requested_scored_matches": snapshot["scored_matches"],
               "bcore_complete": rust["complete"], "bcore_coverage_status": rust["coverage_status"],
               "bcore_stages": rust["stages"], "bcore_feature_sources": rust["feature_sources"]}
        row["scored_matches"] = snapshot["scored_matches"] and sequence["matches"] and all(w["scored_matches"] for w in watches)
        row["matches"] = False if not row["scored_matches"] else None
        row["unscored_row_fields"] = [{"path": [key], "scored": False, "reason": "BCore diagnostics; not a native parity score"}
            for key in sorted(rust) if key not in
            {"schema", "observation_contract", "request", "phase", "spec", "seed", "game_time",
             "snapshot", "watch", *NATIVE_INPUT_FILES}]
        rows.append(row)
    return {"schema": REPLAY_SCHEMA, "observation_contract": OBSERVATION_CONTRACT,
            "capture": str(directory), **identity, "comparator_sha256": sha(Path(__file__)),
            "rust_output_sha256": sha(rust_path), "requests": rows,
            "scored_matches": all(row["scored_matches"] for row in rows),
            "matches": False if any(not row["scored_matches"] for row in rows) else None,
            "limitations": ["Requested snapshots select the returned object; ordered watch snapshots select canonical holders. Missing/duplicate/reordered/wrong-position observations are rejected; explicit absence is scored.",
                             "Every state, biome, heightmap, light and logical metadata difference is retained, including length tails. Examples are only previews. LIGHT/SPAWN and all other status differences are never normalized.",
                             "NBT saved Status and the later native observed persisted status are compared separately. Native work can advance between those observations.",
                             "Light scoring covers serialized layer presence/bytes, including pre-LIGHT storage. Lazy-zero versus absent storage requires the independent layer oracle; isLightOn has no BCore counterpart here.",
                             "Logical structure data, saved BE payloads/keepPacked, ordered saved ticks and per-section postprocessing are scored. Omitted native tick lists denote empty queues. Numeric NBT widths and compound wire order remain unscored.",
                             "Unsupported/unknown fields are inventoried as unscored. scored_matches covers only measured fields; matches=null cannot assert whole-snapshot or FULL parity. Partial implementation coverage is retained.",
                             "Feature source order is the accumulated reported sequence; write attribution is bounded by request_exit, not a separate watch timestamp. Bootstrap calls add no saves. Entity/world registration and tick execution remain outside these snapshot claims."]}


def native_diff(expected_dir, actual_dir):
    left, right = load_events(expected_dir), load_events(actual_dir)
    configs = [json.loads((d / "config.json").read_text()) for d in (expected_dir, actual_dir)]
    if configs[0]["seed"] != configs[1]["seed"]:
        raise ValueError("native history comparison requires the same seed")
    states = json.loads((expected_dir / "block-states.json").read_text())
    if states != json.loads((actual_dir / "block-states.json").read_text()):
        raise ValueError("native state registries differ")
    registry = {state_key(s): s["id"] for s in states}
    snapshots = []
    for events in (left, right):
        last = next(e for e in reversed(events) if e["event"] == "request_exit")
        snapshots.append({tuple(s["pos"]): s for s in last["watch"] if not s.get("absent")})
    rows = []
    for pos in sorted(snapshots[0].keys() & snapshots[1].keys()):
        decoded = [decode_snapshot(d, snapshot[pos], registry) for d, snapshot in zip((expected_dir, actual_dir), snapshots)]
        x, z = pos
        first_y = decoded[0]["min_y"]
        blocks = mismatch(decoded[0]["states"], decoded[1]["states"],
                          lambda i: [x * 16 + i % 16, first_y + i // 256, z * 16 + i // 16 % 16])
        if blocks.get("first"):
            for side in ("expected", "actual"):
                blocks["first"][side + "_state"] = states[blocks["first"][side]]
        rows.append({"pos": pos, "expected_nbt": snapshots[0][pos]["nbt"], "actual_nbt": snapshots[1][pos]["nbt"],
                     "raw_nbt_equal": snapshots[0][pos]["nbt"] == snapshots[1][pos]["nbt"],
                     "states": blocks, "biomes_equal": decoded[0]["biomes"] == decoded[1]["biomes"],
                     "nbt_top_level_differences": [k for k in decoded[0]["nbt"].keys() | decoded[1]["nbt"].keys()
                                                    if decoded[0]["nbt"].get(k) != decoded[1]["nbt"].get(k)]})
    sources = [[e["pos"] for e in events if e["event"] == "stage_enter" and e["status"] == "minecraft:features"]
               for events in (left, right)]
    same_requests = configs[0]["requests"] == configs[1]["requests"] and configs[0]["bootstrap"] == configs[1]["bootstrap"]
    return {"expected": str(expected_dir), "actual": str(actual_dir), "seed": configs[0]["seed"],
            "same_requests": same_requests, "same_source_set": sorted(sources[0]) == sorted(sources[1]),
            "sources": sources, "source_order": mismatch(*sources, lambda i: i + 1), "chunks": rows,
            "provenance_sha256": [sha(d / "provenance.json") for d in (expected_dir, actual_dir)]}


def plan_from_events(config, events, identity):
    validate_stages(events)
    if any(type(e.get("seq")) is not int or e["seq"] != index for index, e in enumerate(events, 1)):
        raise ValueError("non-contiguous observation sequence")
    bootstrap = [{"pos": e["pos"], "status": e["status"].removeprefix("minecraft:"), "create": e["create"]} for e in events
                 if e["event"] == "native_request" and e["api"] == "server/level/ServerChunkCache.getChunk"
                 and e["phase"] == "bootstrap"]
    times = [e.get("game_time") for e in events if e["event"] in ("bootstrap_complete", "request_exit", "oracle_complete")]
    if not times or any(type(time) is not int or time != 0 for time in times):
        raise ValueError("fresh-world replay requires recorded zero game time at every observation boundary")
    if type(events[-1].get("gameplay_ticks")) is not int or events[-1]["gameplay_ticks"] != 0:
        raise ValueError("generation history includes unknown/nonzero gameplay ticks")
    boundaries = [e for e in events if e["event"] == "bootstrap_complete"]
    if len(boundaries) != 1 or boundaries[0].get("mode") != config["bootstrap"]:
        raise ValueError("missing, duplicate or mismatched bootstrap boundary")
    seed = exact_i64(config["seed"], "seed")
    if type(boundaries[0].get("native_seed")) is not int or boundaries[0]["native_seed"] != seed:
        raise ValueError("native bootstrap seed differs from exact config seed")
    plan = {"schema": REPLAY_SCHEMA, "observation_contract": OBSERVATION_CONTRACT,
            "game_time": "0", "seed": config["seed"], "bootstrap": config["bootstrap"],
            "bootstrap_replayed": True, "bootstrap_requests": bootstrap,
            "requests": config["requests"], "watch": config["watch"], **identity,
            "contract": "Recorded synchronous fresh-overworld getChunk requests at game time zero. Bootstrap calls have no added save observations. Scripted watches select canonical holders before the selected returned object's save. Imposters unwrap only for serialization/class. Status variation, source order and Partial coverage remain explicit."}
    validate_replay_plan(plan)
    if any(spec.get("create", True) is not True for spec in plan["requests"]):
        raise ValueError("native HistoryProbe scripted requests always use create=true")
    exits = [e for e in events if e["event"] == "request_exit"]
    if len(exits) != len(plan["requests"]):
        raise ValueError("native request count differs from config")
    for index, (event, spec) in enumerate(zip(exits, plan["requests"])):
        # HistoryProbe reads scripted positions through Gson's Map -> Double.
        # Only those exactly integral i32 values have the config's integer identity.
        observed = event.get("spec", {})
        coordinates = observed.get("pos")
        if (not isinstance(coordinates, list) or len(coordinates) != 2
                or any(type(v) not in (int, float) or v != expected
                       for v, expected in zip(coordinates, spec["pos"]))
                or {k: v for k, v in observed.items() if k != "pos"} != {k: v for k, v in spec.items() if k != "pos"}
                or event.get("phase") != "requests" or type(event.get("request")) is not int
                or event["request"] != index):
            raise ValueError(f"native request {index} differs from config/order")
    return plan


def replay_plan(directory):
    identity = native_input_identity(directory)
    config = json.loads((directory / "config.json").read_text(encoding="utf-8"))
    return plan_from_events(config, load_events(directory), identity)


def height_history(directory):
    result = []
    for event in load_events(directory):
        if event["event"] != "request_exit":
            continue
        snapshot = event["snapshot"]
        extra = json.loads(blob(directory, snapshot["extra"]))
        result.append({"request": event["request"], "spec": event["spec"], "status": snapshot["status"],
                       "chunk_class": extra["chunk_class"], "live_map_keys": list(extra["heightmaps_live"]),
                       "extra_blob": snapshot["extra"]})
    return result


def request_metadata(directory):
    events = load_events(directory)
    validate_stages(events)
    result = []
    for event in events:
        if event["event"] != "request_exit":
            continue
        snapshot = event["snapshot"]
        nbt = NbtReader(blob(directory, snapshot["nbt"])).read()
        extra = json.loads(blob(directory, snapshot["extra"]))
        result.append({"request": event["request"], "spec": event["spec"],
                       "snapshot_status": snapshot["status"], "chunk_class": extra["chunk_class"],
                       "nbt_keys": list(nbt), "metadata": {key: nbt[key] for key in
                           ("structures", "block_entities", "entities", "block_ticks", "fluid_ticks", "PostProcessing", "UpgradeData")
                           if key in nbt}, "level_entities": extra["level_entities"],
                       "nbt_blob": snapshot["nbt"]})
    return result


def feature_changes(directory, position):
    """Attribute even direct section writes using native pre/post snapshots."""
    x, y, z = map(int, position.split(","))
    owner = [x >> 4, z >> 4]
    events = load_events(directory)
    validate_stages(events)
    registry = {state_key(s): s["id"] for s in json.loads((directory / "block-states.json").read_text())}
    before = {}
    changes = []
    for event in events:
        if event["event"] != "feature_snapshot":
            continue
        snapshot = next((s for s in event["chunks"] if s["pos"] == owner), None)
        if snapshot is None:
            continue
        data = decode_snapshot(directory, snapshot, registry)
        index = (y - data["min_y"]) * 256 + (z & 15) * 16 + (x & 15)
        if not 0 <= index < len(data["states"]):
            raise ValueError("feature witness lies outside the captured height range")
        value = data["states"][index]
        key = event["stage_id"], event["feature"]
        if event["boundary"] == "before":
            before[key] = value
        elif before[key] != value:
            changes.append({"source": event["source"], "feature": event["feature"],
                            "before": before[key], "after": value, "event": event["seq"]})
    return {"position": [x,y,z], "changes": changes}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("summary", "verify", "compare", "native-diff", "replay-plan", "height-history", "request-metadata", "feature-changes"))
    parser.add_argument("capture", type=Path)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--other", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--position", help="absolute x,y,z witness for feature-changes")
    args = parser.parse_args()
    if args.operation == "compare":
        result = compare(args.capture, args.rust)
    elif args.operation == "native-diff":
        result = native_diff(args.capture, args.other)
    elif args.operation == "replay-plan":
        result = replay_plan(args.capture)
    elif args.operation == "height-history":
        result = height_history(args.capture)
    elif args.operation == "request-metadata":
        result = request_metadata(args.capture)
    elif args.operation == "feature-changes":
        if not args.position:
            parser.error("feature-changes requires --position=x,y,z")
        result = feature_changes(args.capture, args.position)
    else:
        result = summarize(args.capture, args.operation == "verify")
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output:
        with args.output.open("x", encoding="utf-8") as stream:
            stream.write(rendered)
        print(args.output)
    else:
        print(rendered)


if __name__ == "__main__":
    main()
