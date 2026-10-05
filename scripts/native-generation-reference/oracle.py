"""Verify, decode and compare actual-engine history captures without altering fixtures."""

import argparse
from collections import Counter
import gzip
import hashlib
import io
import json
from pathlib import Path
import struct


class NbtReader:
    """Uncompressed named-root NBT. Original blob bytes remain the typed authority."""

    def __init__(self, data):
        self.stream = io.BytesIO(data)

    def take(self, size):
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
    return hashlib.sha256(path.read_bytes()).hexdigest()


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
    if len(expected) != len(actual):
        return {"expected_length": len(expected), "actual_length": len(actual)}
    first = None
    count = 0
    pairs = Counter()
    examples = []
    for i, (a, b) in enumerate(zip(expected, actual)):
        if a != b:
            count += 1
            pairs[str(a), str(b)] += 1
            if first is None:
                first = {"pos": coordinates(i), "expected": a, "actual": b}
            if len(examples) < 16:
                examples.append({"pos": coordinates(i), "expected": a, "actual": b})
    return {"mismatches": count, "total": len(expected), "first": first,
            "examples": examples,
            "pairs": [{"expected": a, "actual": b, "count": n} for (a, b), n in pairs.most_common(20)]}


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
    result["scored"] = True
    native = {s["Y"]: s for s in nbt["sections"]}
    expected_min = nbt["yPos"] - 1
    expected_count = sum("block_states" in s for s in nbt["sections"]) + 2
    actual_min = actual["min_section_y"]
    sections = {actual_min + i: s for i, s in enumerate(actual["sections"])}
    result["geometry_matches"] = actual_min == expected_min and len(sections) == expected_count
    result["layers"] = []
    ys = set(range(expected_min, expected_min + expected_count)) | sections.keys() | native.keys()
    for y in sorted(ys):
        section = sections.get(y, {})
        for kind, tag in (("sky", "SkyLight"), ("block", "BlockLight")):
            expected_layer = native.get(y, {}).get(tag)
            actual_layer = section.get(kind)
            if section.get(kind + "_empty", False):
                if actual_layer is None or len(actual_layer) != 2048 or any(actual_layer):
                    raise ValueError(f"invalid lazy-zero {kind} layer at section {y}")
                actual_layer = None
            layer = {"y": y, "kind": kind,
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
    result = {"scored": True, "mismatches": 0, "examples": []}

    def difference(path, left, right):
        result["mismatches"] += 1
        if len(result["examples"]) < 16:
            result["examples"].append({"path": path,
                "expected_present": left is not absent, "actual_present": right is not absent,
                "expected": None if left is absent else left,
                "actual": None if right is absent else right})

    def visit(left, right, path):
        if isinstance(left, dict) and isinstance(right, dict):
            for key in sorted(left.keys() | right.keys()):
                visit(left.get(key, absent), right.get(key, absent), path + [key])
        elif isinstance(left, list) and isinstance(right, list):
            for index in range(max(len(left), len(right))):
                visit(left[index] if index < len(left) else absent,
                      right[index] if index < len(right) else absent, path + [index])
        elif isinstance(left, float) and isinstance(right, float):
            if struct.pack(">d", left) != struct.pack(">d", right):
                difference(path, left, right)
        elif left is absent or right is absent or left != right:
            difference(path, left, right)

    visit(expected, actual, [])
    result["matches"] = result["mismatches"] == 0
    return result


def compare_generated_metadata(nbt, actual):
    result = {}
    structures = actual.get("structures")
    result["structures"] = ({"scored": False} if structures is None else nested_mismatch(
        nbt.get("structures"), {"starts": structures["starts"], "References": structures["references"]}))

    def entities_by_position(rows, native):
        indexed = {}
        for row in rows:
            data = row if native else row["nbt"]
            position = [data[axis] for axis in ("x", "y", "z")] if native else row["pos"]
            key = ",".join(str(int(v)) for v in position)
            if key in indexed:
                raise ValueError(f"duplicate block entity at {key}")
            # keepPacked is LevelChunk's serialization envelope, not the block
            # entity's saved payload. Expose it separately instead of hiding it.
            indexed[key] = {key: value for key, value in data.items() if key != "keepPacked"}
        return indexed

    result["block_entity_payloads"] = ({"scored": False} if "block_entities" not in actual else
        nested_mismatch(entities_by_position(nbt.get("block_entities", []), True),
                        entities_by_position(actual["block_entities"], False)))
    result["block_entity_packing"] = {"scored": False, "native_counts": dict(Counter(
        str(row["keepPacked"]) if "keepPacked" in row else "absent" for row in nbt.get("block_entities", [])))}

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


def compare(directory, rust_path):
    events = load_events(directory)
    validate_stages(events)
    expected = [e for e in events if e["event"] == "request_exit"]
    actual = [row for line in rust_path.read_text(encoding="utf-8").splitlines()
              if (row := json.loads(line)).get("phase", "requests") == "requests"]
    if len(expected) != len(actual):
        raise ValueError(f"request count mismatch: native {len(expected)}, BCore {len(actual)}")
    states = json.loads((directory / "block-states.json").read_text(encoding="utf-8"))
    registry = {state_key(state): state["id"] for state in states}
    rows = []
    for native, rust in zip(expected, actual):
        if native["spec"] != rust["spec"]:
            raise ValueError(f"unmatched request histories: {native['spec']} versus {rust['spec']}")
        snapshot = native["snapshot"]
        decoded = decode_snapshot(directory, snapshot, registry)
        x, z = snapshot["pos"]
        min_y = decoded["min_y"]
        blocks = mismatch(decoded["states"], rust["snapshot"]["states"],
                          lambda i: [x * 16 + i % 16, min_y + i // 256, z * 16 + i // 16 % 16])
        if blocks.get("first"):
            for side in ("expected", "actual"):
                blocks["first"][side + "_state"] = states[blocks["first"][side]]
            blocks["first"]["native_writes"] = locate_writes(events, blocks["first"]["pos"], native["seq"])
        biomes = mismatch(decoded["biomes"], rust["snapshot"]["biomes"],
                          lambda i: [x * 4 + i % 4, min_y // 4 + i // 16, z * 4 + i // 4 % 4])
        native_sources = [e["pos"] for e in events if e["event"] == "stage_enter"
                          and e["status"] == "minecraft:features" and e["seq"] < native["seq"]]
        rust_sources = [s["source"] for s in rust["feature_sources"]]
        sequence = mismatch(native_sources, rust_sources, lambda i: i + 1)
        wg_maps = {}
        native_maps = decoded["extra"]["heightmaps_live"]
        rust_maps = rust["snapshot"].get("worldgen_heightmaps") or {}
        height_palette = range(min_y, min_y + len(decoded["states"]) // 256 + 1)
        for name in ("WORLD_SURFACE_WG", "OCEAN_FLOOR_WG"):
            present = name in native_maps
            actual_present = name in rust_maps
            result = {"native_present": present, "bcore_present": actual_present,
                      "presence_matches": present == actual_present}
            if present and actual_present:
                heights = unpack(height_palette, native_maps[name], 256, 1)
                result["values"] = mismatch(heights, rust_maps[name],
                    lambda i: [x * 16 + i % 16, z * 16 + i // 16])
            wg_maps[name] = result
        light = compare_serialized_light(decoded["nbt"], rust["snapshot"].get("light"))
        metadata = compare_generated_metadata(decoded["nbt"], rust["snapshot"])
        rows.append({"request": native["request"], "spec": native["spec"], "native_event": native["seq"],
                     "native_nbt": snapshot["nbt"], "bcore_complete": rust["complete"],
                     "states": blocks, "biomes": biomes, "source_order": sequence,
                     "worldgen_heightmaps": wg_maps,
                     "serialized_light": light,
                     "generated_metadata": metadata,
                     "native_effect_counts": {name: len(decoded["nbt"].get(name, [])) for name in
                                              ("block_entities", "entities", "block_ticks", "fluid_ticks")},
                     "bcore_stages": rust["stages"]})
    return {"capture": str(directory), "native_provenance_sha256": sha(directory / "provenance.json"),
            "comparator_sha256": sha(Path(__file__)),
            "rust_output_sha256": sha(rust_path), "requests": rows,
            "limitations": ["Scores states, quart biomes, source order and WG heightmap presence/values. Native FULL conversion removes WG maps; an unconverted BCore prototype remains a presence mismatch.",
                             "Scores serialized sky/block light when a BCore snapshot exists, including pre-LIGHT storage. Native disk serialization omits lazy-zero layers; lazy/null distinctions need their independent oracle.",
                            "Scores logical structure starts/references, saved block-entity payloads keyed by position, and per-section postprocessing order/duplicates. Numeric NBT widths and compound wire order remain independently tested; LevelChunk keepPacked envelopes are explicitly unscored.",
                            "Entity creation, tick-container lifecycle and other generated effects remain diagnostic payload, not a passing parity claim."]}


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


def replay_plan(directory):
    config = json.loads((directory / "config.json").read_text())
    events = load_events(directory)
    validate_stages(events)
    bootstrap = [{"pos": e["pos"], "status": e["status"].removeprefix("minecraft:")} for e in events
                 if e["event"] == "native_request" and e["api"] == "server/level/ServerChunkCache.getChunk"
                 and e["phase"] == "bootstrap" and e["create"]]
    return {"schema": 1, "seed": config["seed"], "bootstrap": config["bootstrap"],
            "bootstrap_replayed": True, "bootstrap_requests": bootstrap,
            "requests": config["requests"], "watch": config["watch"],
            "native_provenance_sha256": sha(directory / "provenance.json"),
            "native_events_sha256": sha(directory / "events.jsonl"),
            "contract": "Recorded synchronous overworld requests, including repeated native bootstrap requests. Verify resulting native FEATURES sequence separately; this does not assert arbitrary async ticket-history parity."}


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
