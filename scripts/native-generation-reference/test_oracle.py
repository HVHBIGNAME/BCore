"""Portable actual-engine evidence and corruption/history rejection checks."""

from collections import Counter
from copy import deepcopy
import gzip
import hashlib
import io
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import warnings
import zipfile

from oracle import (
    NATIVE_INPUT_FILES, OBSERVATION_CONTRACT, REPLAY_SCHEMA, STATUSES, NbtReader,
    compare, compare_generated_metadata, compare_serialized_light, decode_snapshot,
    load_events, mismatch, native_diff, nested_mismatch, plan_from_events, replay_plan,
    sha, state_key, summarize, unpack, validate_replay_plan, validate_replay_rows, validate_stages,
)
from replay_bcore import (
    HISTORY_CONTRACT, HISTORY_PROGRAM, OBSERVER, REQUIRED_TOOLS, hashes, sources,
    tool_sources, validate_archive, validate_binary_contract, validate_reuse_build, write_archive,
    validate_source_dependencies,
)

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def extract_fixture(name, destination):
    index = json.loads((HERE / "fixtures/index.json").read_text())
    expected = next(f for f in index["fixtures"] if f["file"] == name)
    if sha(HERE / "fixtures" / name) != expected["sha256"]:
        raise ValueError(f"fixture archive digest mismatch: {name}")
    with zipfile.ZipFile(HERE / "fixtures" / name) as archive:
        manifest = json.loads(archive.read("fixture.json"))
        for name, digest in manifest["files_sha256"].items():
            path = destination / name
            if not path.resolve().is_relative_to(destination.resolve()):
                raise ValueError("fixture path escapes extraction directory")
            data = archive.read(name)
            if hashlib.sha256(data).hexdigest() != digest:
                raise ValueError(f"fixture digest mismatch: {name}")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    return manifest


class DecoderTests(unittest.TestCase):
    def test_palettes_do_not_straddle_native_long_padding(self):
        palette = list(range(17))  # five bits; twelve entries per long, four padding bits
        words = [(1 << 63) | sum((i % 17) << (5 * i) for i in range(12)), 16 | (3 << 5)]
        self.assertEqual(unpack(palette, words, 14, 4), list(range(12)) + [16, 3])
        with self.assertRaises(ValueError):
            unpack(palette, words[:1], 14, 4)

    def test_native_numeric_precision_and_truncation_are_not_normalized(self):
        data = io.BytesIO()
        data.write(b"\x0a\x00\x00\x04\x00\x04seed")
        data.write(struct.pack(">q", 846692123413862008))
        data.write(b"\x06\x00\x01d" + struct.pack(">d", -0.0) + b"\x00")
        decoded = NbtReader(data.getvalue()).read()
        self.assertEqual(decoded["seed"], 846692123413862008)
        self.assertEqual(struct.pack(">d", decoded["d"]), b"\x80" + b"\0" * 7)
        with self.assertRaises(ValueError):
            NbtReader(data.getvalue()[:-1]).read()
        with self.assertRaises(ValueError):
            NbtReader(data.getvalue() + b"extra").read()


class SerializedLightTests(unittest.TestCase):
    def setUp(self):
        self.nbt = {"yPos": -4, "sections": [{"Y": -4, "block_states": {}, "SkyLight": [0] * 2048}]}
        self.actual = {"min_section_y": -5, "sections": [
            {"sky": None, "block": None},
            {"sky": [0] * 2048, "block": None, "sky_empty": False},
            {"sky": None, "block": None},
        ]}

    def test_pre_light_materialized_zero_is_compared(self):
        result = compare_serialized_light(self.nbt, self.actual)
        self.assertFalse(result["native_is_light_on"])
        self.assertTrue(result["matches"])
        self.actual["sections"][1]["sky"][19] = 255
        self.assertEqual(compare_serialized_light(self.nbt, self.actual)["byte_mismatches"], 1)

    def test_lazy_zero_and_null_omit_disk_tag_but_materialized_zero_does_not(self):
        self.actual["sections"][1]["sky_empty"] = True
        self.assertEqual(compare_serialized_light(self.nbt, self.actual)["presence_mismatches"], 1)
        del self.nbt["sections"][0]["SkyLight"]
        self.assertTrue(compare_serialized_light(self.nbt, self.actual)["matches"])
        self.actual["sections"][1] = {"sky": None, "block": None}
        self.assertTrue(compare_serialized_light(self.nbt, self.actual)["matches"])

    def test_missing_snapshot_and_truncated_or_shifted_layers_cannot_pass(self):
        self.assertFalse(compare_serialized_light(self.nbt, None)["scored"])
        self.actual["sections"][1]["sky"].pop()
        result = compare_serialized_light(self.nbt, self.actual)
        self.assertFalse(result["matches"])
        self.assertEqual(result["length_mismatches"], 1)
        self.actual["sections"].pop()
        self.assertFalse(compare_serialized_light(self.nbt, self.actual)["geometry_matches"])
        self.actual["min_section_y"] -= 1
        self.assertGreater(compare_serialized_light(self.nbt, self.actual)["presence_mismatches"], 0)

    def test_invalid_lazy_zero_flag_is_rejected(self):
        self.actual["sections"][1]["sky_empty"] = True
        self.actual["sections"][1]["sky"][0] = 1
        with self.assertRaises(ValueError):
            compare_serialized_light(self.nbt, self.actual)


class MetadataTests(unittest.TestCase):
    def test_missing_fields_and_ordered_duplicates_are_not_normalized(self):
        self.assertFalse(nested_mismatch({"seed": 0}, {})["matches"])
        self.assertFalse(nested_mismatch([1, 2, 1], [1, 1, 2])["matches"])
        self.assertFalse(nested_mismatch([1, 1], [1])["matches"])
        self.assertFalse(nested_mismatch(-0.0, 0.0)["matches"])
        self.assertEqual(nested_mismatch({"seed": 846692123413862008},
                                        {"seed": 846692123413862009})["mismatches"], 1)
        self.assertFalse(nested_mismatch({"flag": 1}, {"flag": True})["matches"])
        self.assertFalse(nested_mismatch(1, 1.0)["matches"])

    def test_all_differences_and_length_tails_survive_preview_limits(self):
        result = mismatch([0] * 40, [1] * 42, lambda i: i)
        self.assertEqual(result["mismatches"], 42)
        self.assertEqual(len(result["differences"]), 42)
        self.assertEqual(len(result["examples"]), 16)
        self.assertEqual((result["expected_length"], result["actual_length"]), (40, 42))
        self.assertFalse(result["differences"][-1]["expected_present"])
        self.assertEqual(result["differences"][-1]["pos"], 41)
        result = nested_mismatch(list(range(40)), [])
        self.assertEqual(len(result["differences"]), 40)
        self.assertFalse(result["differences"][-1]["actual_present"])

    def test_block_entities_are_position_keyed_but_all_payload_fields_are_scored(self):
        a = {"id": "minecraft:brushable_block", "x": -16, "y": -48, "z": 0, "LootTableSeed": 846692123413862008}
        b = {"id": "minecraft:brushable_block", "x": -15, "y": -48, "z": 0, "LootTableSeed": -17}
        nbt = {"block_entities": [a | {"keepPacked": 0}, b]}
        actual = {"block_entities": [{"pos": [-15, -48, 0], "nbt": b}, {"pos": [-16, -48, 0], "nbt": a}]}
        result = compare_generated_metadata(nbt, actual)
        self.assertTrue(result["block_entity_payloads"]["matches"])
        self.assertFalse(result["block_entity_packing"]["scored"])
        actual["block_entities"][1]["nbt"] = a | {"LootTableSeed": 0}
        self.assertEqual(compare_generated_metadata(nbt, actual)["block_entity_payloads"]["mismatches"], 1)
        actual["block_entities"].append(actual["block_entities"][0])
        with self.assertRaises(ValueError):
            compare_generated_metadata(nbt, actual)

    def test_native_section_postprocessing_preserves_negative_y_order_and_duplicates(self):
        nbt = {"yPos": -4, "sections": [{"block_states": {}} for _ in range(24)],
               "PostProcessing": [[] for _ in range(24)]}
        nbt["PostProcessing"][0] = [0x231, 0x231]
        nbt["PostProcessing"][23] = [0xfff]
        actual = {"postprocessing": [[1, -61, 2], [15, 319, 15], [1, -61, 2]]}
        self.assertTrue(compare_generated_metadata(nbt, actual)["postprocessing"]["matches"])
        actual["postprocessing"].pop()
        self.assertFalse(compare_generated_metadata(nbt, actual)["postprocessing"]["matches"])
        actual["postprocessing"].append([16, -61, 2])
        with self.assertRaises(ValueError):
            compare_generated_metadata(nbt, actual)

    def test_save_boundaries_score_packing_and_ordered_saved_ticks(self):
        entity = {"id": "minecraft:sculk_sensor", "x": 1, "y": 80, "z": 1, "keepPacked": 0}
        first = {"i": "minecraft:stone", "x": 4, "y": 80, "z": 4, "t": 0, "p": 0}
        second = first | {"x": 5, "p": -1}
        nbt = {"block_entities": [entity], "block_ticks": [first, second], "fluid_ticks": []}
        actual = {"snapshot_boundary": "level_save",
                  "block_entities": [{"pos": [1, 80, 1], "nbt": dict(entity)}],
                  "saved_ticks": {"block_ticks": [first, second], "fluid_ticks": []}}
        result = compare_generated_metadata(nbt, actual)
        self.assertTrue(result["block_entity_packing"]["matches"])
        self.assertTrue(result["saved_ticks"]["matches"])
        del actual["block_entities"][0]["nbt"]["keepPacked"]
        result = compare_generated_metadata(nbt, actual)
        self.assertTrue(result["block_entity_payloads"]["matches"])
        self.assertEqual(result["block_entity_packing"]["mismatches"], 1)
        actual["saved_ticks"]["block_ticks"] = [second, first]
        self.assertFalse(compare_generated_metadata(nbt, actual)["saved_ticks"]["matches"])
        actual["saved_ticks"]["block_ticks"] = [first | {"t": -5}, second]
        self.assertEqual(compare_generated_metadata(nbt, actual)["saved_ticks"]["mismatches"], 1)
        actual["saved_ticks"]["block_ticks"] = [first, first, second]
        self.assertFalse(compare_generated_metadata(nbt, actual)["saved_ticks"]["matches"])

    def test_claimed_save_boundary_must_supply_its_observations(self):
        with self.assertRaises(ValueError):
            compare_generated_metadata({}, {"snapshot_boundary": "level_save", "block_entities": []})
        with self.assertRaises(ValueError):
            compare_generated_metadata({}, {"snapshot_boundary": "unverified"})
        legacy = compare_generated_metadata({}, {"block_entities": []})
        self.assertFalse(legacy["block_entity_packing"]["scored"])
        self.assertFalse(legacy["saved_ticks"]["scored"])


class StagePrefixTests(unittest.TestCase):
    def test_unfinished_native_ticket_work_requires_an_exact_terminal_declaration(self):
        entry = {"seq": 1, "event": "stage_enter", "pos": [1,0], "status": "minecraft:full", "request": 5, "thread": "worker"}
        before = {"event": "stage_snapshot", "stage_id": 1, "boundary": "before"}
        terminal = {"event": "oracle_complete"}
        with self.assertRaises(ValueError):
            validate_stages([entry, before, terminal])
        terminal["inflight_stages"] = [{k:v for k,v in entry.items() if k not in ("seq","event")} | {"stage_id": 1}]
        _, pending = validate_stages([entry, before, terminal])
        self.assertEqual(len(pending), 1)
        terminal["inflight_stages"][0]["status"] = "minecraft:light"
        with self.assertRaises(ValueError):
            validate_stages([entry, before, terminal])

    def test_completed_stage_cannot_also_be_declared_in_flight(self):
        entry = {"seq": 1, "event": "stage_enter", "pos": [1,0], "status": "minecraft:full", "request": 5, "thread": "worker"}
        terminal = {"event": "oracle_complete", "inflight_stages": [{"stage_id":1}]}
        with self.assertRaises(ValueError):
            validate_stages([entry, {"event":"stage_exit", "stage_id":1}, terminal])

    def test_missing_terminal_and_orphaned_snapshots_cannot_pass_replay_validation(self):
        with self.assertRaises(ValueError):
            validate_stages([])
        with self.assertRaises(ValueError):
            validate_stages([{"event": "request_exit"}])
        with self.assertRaises(ValueError):
            validate_stages([{"event": "stage_snapshot", "stage_id": 99, "boundary": "after"},
                             {"event": "oracle_complete"}])


def contract_plan():
    return {"schema": REPLAY_SCHEMA, "observation_contract": OBSERVATION_CONTRACT,
            "seed": "846692123413862008", "game_time": "0", "bootstrap": "before_spawn",
            "bootstrap_replayed": True, "bootstrap_requests": [],
            "requests": [{"pos": [0, 0], "status": "light"}], "watch": [[0, 0], [9, 9]],
            **{field: chr(ord("a") + i) * 64 for i, field in enumerate(NATIVE_INPUT_FILES)}}


def implementation_stages():
    return [{"status": "minecraft:" + status, "state": "Partial" if status == "full" else "Complete",
             "attempts": 1, "missing": ["world registration"] if status == "full" else []}
            for status in STATUSES]


def contract_row(plan, snapshot, watches):
    return {key: plan[key] for key in ("schema", "observation_contract", "seed", "game_time", *NATIVE_INPUT_FILES)} | {
        "phase": "requests", "request": 0, "spec": deepcopy(plan["requests"][0]),
        "snapshot": snapshot, "watch": watches, "complete": False,
        "coverage_status": snapshot["persisted_status"], "stages": implementation_stages(), "feature_sources": []}


class ReplayContractTests(unittest.TestCase):
    def test_exact_i64_schema_and_trace_contract(self):
        plan = contract_plan()
        for seed in ("-9223372036854775808", "9223372036854775807", "+846692123413862008"):
            plan["seed"] = seed
            plan["feature_trace"] = {"source": [-1, 0], "feature": "minecraft:patch_grass_forest"}
            validate_replay_plan(plan)
        for field, value in (("schema", 2), ("schema", 3.0), ("schema", True),
                             ("observation_contract", "holder_only_v2"), ("seed", 846692123413862008),
                             ("seed", "9223372036854775808"), ("seed", " 1"),
                             ("game_time", "1"), ("game_time", 0), ("game_time", False),
                             ("native_events_sha256", "A" * 64), ("native_config_sha256", None)):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                validate_replay_plan(contract_plan() | {field: value})

    def test_bad_requests_and_duplicate_watches_are_rejected_before_replay(self):
        plan = contract_plan()
        for position in ([True, 0], [0.0, 0], [0], [0, 0, 1], [1 << 31, 0]):
            with self.subTest(position=position), self.assertRaises(ValueError):
                validate_replay_plan(plan | {"watch": [position]})
        for request in ({"pos": [0, 0], "status": "unknown"},
                        {"pos": [0, 0], "status": "light", "create": "true"}):
            with self.subTest(request=request), self.assertRaises(ValueError):
                validate_replay_plan(plan | {"requests": [request]})
        with self.assertRaisesRegex(ValueError, "duplicate watch"):
            validate_replay_plan(plan | {"watch": [[0, 0], [0, 0]]})
        with self.assertRaisesRegex(ValueError, "feature_trace"):
            validate_replay_plan(plan | {"feature_trace": {"source": [0, 0], "feature": ""}})

    def test_native_bootstrap_retains_order_duplicates_create_and_seed(self):
        config = contract_plan() | {"bootstrap": "native", "requests": []}
        calls = [{"event": "native_request", "api": "server/level/ServerChunkCache.getChunk",
                  "phase": "bootstrap", "pos": [1, -1], "status": "minecraft:light", "create": create}
                 for create in (False, True, True)]
        events = calls + [{"event": "bootstrap_complete", "mode": "native", "game_time": 0,
                           "native_seed": int(config["seed"])},
                          {"event": "oracle_complete", "game_time": 0, "gameplay_ticks": 0}]
        events = [event | {"seq": index} for index, event in enumerate(events, 1)]
        identity = {field: config[field] for field in NATIVE_INPUT_FILES}
        plan = plan_from_events(config, events, identity)
        self.assertEqual([r["create"] for r in plan["bootstrap_requests"]], [False, True, True])
        self.assertEqual(plan["bootstrap_requests"][1], plan["bootstrap_requests"][2])
        self.assertEqual({field: plan[field] for field in identity}, identity)
        rows = [{key: plan[key] for key in ("schema", "observation_contract", "seed", "game_time", *identity)} | {
            "phase": "bootstrap", "request": i - 3, "spec": spec, "snapshot": None, "watch": [],
            "complete": None, "coverage_status": None, "stages": None, "feature_sources": []}
            for i, spec in enumerate(plan["bootstrap_requests"])]
        validate_replay_rows(rows, plan)
        rows[1]["watch"] = [{"pos": [0, 0], "absent": True}]
        with self.assertRaisesRegex(ValueError, "bootstrap calls"):
            validate_replay_rows(rows, plan)
        events[-2]["native_seed"] += 1
        with self.assertRaisesRegex(ValueError, "seed"):
            plan_from_events(config, events, identity)


def nbt_payload(kind, value):
    """Small typed test writer; synthetic contract cases never replace native fixtures."""
    if kind in (1, 3, 4):
        return struct.pack({1: ">b", 3: ">i", 4: ">q"}[kind], value)
    if kind == 8:
        encoded = value.encode("utf-8")
        return struct.pack(">H", len(encoded)) + encoded
    if kind == 9:
        child_kind, items = value
        return struct.pack(">Bi", child_kind, len(items)) + b"".join(nbt_payload(child_kind, item) for item in items)
    if kind == 10:
        return b"".join(bytes([child_kind]) + nbt_payload(8, name) + nbt_payload(child_kind, item)
                        for name, child_kind, item in value) + b"\x00"
    if kind == 12:
        return struct.pack(">i", len(value)) + b"".join(struct.pack(">q", word) for word in value)
    raise ValueError(f"unsupported test tag {kind}")


class SnapshotComparisonTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="native-contract-", dir=ROOT / "target")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "blobs").mkdir()
        self.config = contract_plan()
        self.config = {key: self.config[key] for key in ("seed", "bootstrap", "requests", "watch")}
        self.native_proto, self.proto = self.chunk("spawn", level=False)
        self.native_level, self.level = self.chunk("full", level=True)
        self.absent = {"pos": [9, 9], "absent": True}
        self.events = [
            {"event": "bootstrap_complete", "mode": "before_spawn", "native_seed": int(self.config["seed"]), "game_time": 0},
            {"event": "request_exit", "phase": "requests", "request": 0, "spec": {"pos": [0.0, 0.0], "status": "light"},
             "snapshot": self.native_proto, "watch": [self.native_level, self.absent], "game_time": 0},
            {"event": "oracle_complete", "game_time": 0, "gameplay_ticks": 0},
        ]
        self.events = [event | {"seq": i} for i, event in enumerate(self.events, 1)]
        self.write_capture()
        self.plan = replay_plan(self.root)
        self.row = contract_row(self.plan, self.proto | {"snapshot_origin": "returned_object"},
                                [self.level | {"snapshot_origin": "holder_latest"},
                                 self.absent | {"snapshot_origin": "holder_latest"}])

    def put_blob(self, raw, kind):
        name = hashlib.sha256(raw).hexdigest() + "." + kind + ".gz"
        (self.root / "blobs" / name).write_bytes(gzip.compress(raw))
        return name

    def chunk(self, status, *, level, position=(0, 0), serialized_status=None, extra=None, extra_nbt=()):
        x, z = position
        cls = "net.minecraft.world.level.chunk." + ("LevelChunk" if level else "ProtoChunk")
        packed = 0 if level else 1
        entity = [("id", 8, "minecraft:sculk_sensor"), ("x", 3, x * 16), ("y", 3, 0), ("z", 3, z * 16),
                  ("LootTableSeed", 4, 846692123413862008), ("keepPacked", 1, packed)]
        section = [("Y", 1, 0), ("block_states", 10, [("palette", 9, (10, [[("Name", 8, "minecraft:air")]]))]),
                   ("biomes", 10, [("palette", 9, (8, ["minecraft:plains"]))])]
        entries = [("xPos", 3, x), ("zPos", 3, z), ("yPos", 3, 0),
                   ("Status", 8, "minecraft:" + (serialized_status or status)), ("sections", 9, (10, [section])),
                   ("structures", 10, [("starts", 10, []), ("References", 10, [])]),
                   ("block_entities", 9, (10, [entity])), ("PostProcessing", 9, (9, [(3, [])])),
                   ("isLightOn", 1, 1), *extra_nbt]
        nbt = self.put_blob(b"\x0a\x00\x00" + nbt_payload(10, entries), "nbt")
        maps = {} if level else {key: [0] * 22 for key in ("WORLD_SURFACE_WG", "OCEAN_FLOOR_WG")}
        observed = {"chunk_class": cls, "min_y": 0, "heightmaps_live": maps,
                    "sample_state_ids": [[0, 0], [4095, 0]], **(extra or {})}
        native = {"pos": [x, z], "status": "minecraft:" + status, "nbt": nbt,
                  "extra": self.put_blob(json.dumps(observed).encode(), "json")}
        actual = {"pos": [x, z], "min_y": 0, "states": [0] * 4096, "biomes": ["minecraft:plains"] * 64,
                  "worldgen_heightmaps": None if level else {key: [0] * 256 for key in maps},
                  "runtime_heightmaps": None, "raw_object_kind": "ImposterProto" if level else "Proto",
                  "light": {"min_section_y": -1, "sections": [{"sky": None, "block": None} for _ in range(3)]},
                  "snapshot_boundary": "level_save" if level else "proto_save", "chunk_class": cls,
                  "persisted_status": "minecraft:" + status, "serialized_status": "minecraft:" + status,
                  "structures": {"starts": {}, "references": {}}, "postprocessing": [],
                  "block_entities": [{"pos": [x * 16, 0, z * 16], "nbt": {name: value for name, _, value in entity}}],
                  "saved_ticks": {"block_ticks": [], "fluid_ticks": []}}
        return native, actual

    def write_capture(self):
        (self.root / "config.json").write_text(json.dumps(self.config), encoding="utf-8")
        (self.root / "block-states.json").write_text(json.dumps([
            {"id": 0, "Name": "minecraft:air"}, {"id": 1, "Name": "minecraft:stone"}]), encoding="utf-8")
        (self.root / "events.jsonl").write_text("".join(json.dumps(event) + "\n" for event in self.events), encoding="utf-8")
        (self.root / "provenance.json").write_text(json.dumps({"config_sha256": sha(self.root / "config.json")}), encoding="utf-8")
        (self.root / "capture.json").write_text(json.dumps({
            "events_sha256": sha(self.root / "events.jsonl"), "provenance_sha256": sha(self.root / "provenance.json")}), encoding="utf-8")

    def compare_rows(self, rows=None):
        path = self.root / "chunks.jsonl"
        path.write_text("".join(json.dumps(row) + "\n" for row in ([self.row] if rows is None else rows)), encoding="utf-8")
        return compare(self.root, path)

    def refresh_identity(self):
        self.write_capture()
        plan = replay_plan(self.root)
        self.row.update({field: plan[field] for field in NATIVE_INPUT_FILES})

    def test_returned_old_proto_and_canonical_level_watch_use_the_same_comparator(self):
        result = self.compare_rows()
        row = result["requests"][0]
        self.assertTrue(result["scored_matches"])
        self.assertIsNone(result["matches"])
        self.assertFalse(row["bcore_complete"])
        self.assertEqual(row["bcore_stages"][-1]["state"], "Partial")
        self.assertTrue(row["generated_metadata"]["block_entity_packing"]["matches"])
        self.assertTrue(row["watch"][0]["generated_metadata"]["chunk_lifecycle"]["matches"])
        self.assertFalse(row["watch"][0]["worldgen_heightmaps"]["WORLD_SURFACE_WG"]["bcore_present"])
        self.assertTrue(row["watch"][1]["presence"]["matches"])
        self.assertEqual(row["watch"][1]["scope"], "presence")

    def test_watch_only_state_metadata_and_saved_tick_mismatches_are_retained(self):
        watch = self.row["watch"][0]
        watch["states"][19] = 1
        watch["block_entities"][0]["nbt"]["LootTableSeed"] += 1
        watch["saved_ticks"]["block_ticks"] = [{"i": "minecraft:stone", "x": 0, "y": 0, "z": 0, "t": -5, "p": 0}]
        watch["worldgen_heightmaps"] = {"WORLD_SURFACE_WG": [0] * 256}
        result = self.compare_rows()
        row = result["requests"][0]
        self.assertEqual(row["states"]["mismatches"], 0)
        self.assertFalse(result["scored_matches"])
        self.assertEqual(row["watch"][0]["states"]["differences"][0]["pos"], [3, 0, 1])
        metadata = row["watch"][0]["generated_metadata"]
        self.assertEqual(metadata["block_entity_payloads"]["differences"][0]["expected"], 846692123413862008)
        self.assertFalse(metadata["saved_ticks"]["matches"])
        self.assertFalse(row["watch"][0]["worldgen_heightmaps"]["WORLD_SURFACE_WG"]["presence_matches"])

    def test_light_spawn_and_serialized_status_differences_are_not_normalized(self):
        native, _ = self.chunk("spawn", level=False, serialized_status="light")
        self.events[1]["snapshot"] = native
        self.refresh_identity()
        lifecycle = self.compare_rows()["requests"][0]["generated_metadata"]["chunk_lifecycle"]
        self.assertEqual([d["path"] for d in lifecycle["differences"]], [["serialized_status"]])
        self.row["snapshot"]["persisted_status"] = "minecraft:light"
        lifecycle = self.compare_rows()["requests"][0]["generated_metadata"]["chunk_lifecycle"]
        self.assertEqual({tuple(d["path"]) for d in lifecycle["differences"]}, {("serialized_status",), ("persisted_status",)})

    def test_explicit_presence_disagreement_is_scored_in_both_directions(self):
        present = deepcopy(self.row["watch"][0])
        self.row["watch"][0] = {"pos": [0, 0], "absent": True, "snapshot_origin": "holder_latest"}
        result = self.compare_rows()["requests"][0]["watch"][0]
        self.assertFalse(result["matches"])
        self.assertEqual(result["presence"]["differences"][0]["expected"], True)
        self.events[1]["watch"][0] = {"pos": [0, 0], "absent": True}
        self.row["watch"][0] = present
        self.refresh_identity()
        result = self.compare_rows()["requests"][0]["watch"][0]
        self.assertFalse(result["matches"])
        self.assertEqual(result["presence"]["differences"][0]["expected"], False)

    def test_missing_duplicate_reordered_and_wrong_position_watches_cannot_pass(self):
        original = deepcopy(self.row)
        for watches in (None, [], original["watch"][:1], list(reversed(original["watch"])),
                        [original["watch"][0], original["watch"][0]],
                        [original["watch"][0], {"pos": [9, 9], "snapshot_origin": "holder_latest"}],
                        [original["watch"][0], {"pos": [8, 9], "absent": True, "snapshot_origin": "holder_latest"}]):
            with self.subTest(watches=str(watches)[:60]), self.assertRaises(ValueError):
                self.compare_rows([original | {"watch": watches}])
        self.row["snapshot"]["pos"] = [1, 0]
        with self.assertRaisesRegex(ValueError, "position"):
            self.compare_rows()

    def test_native_duplicate_watches_and_misaddressed_nbt_are_rejected(self):
        self.events[1]["watch"] = [self.native_level, self.native_level]
        self.refresh_identity()
        with self.assertRaisesRegex(ValueError, "duplicate watch"):
            self.compare_rows()
        self.events[1]["watch"] = [self.native_level, self.absent]
        foreign, _ = self.chunk("spawn", level=False, position=(1, 0))
        self.events[1]["snapshot"] = foreign | {"pos": [0, 0]}
        self.refresh_identity()
        with self.assertRaisesRegex(ValueError, "NBT position"):
            self.compare_rows()

    def test_request_schema_phase_order_and_identity_are_checked(self):
        for field, value in (("schema", 2), ("schema", 3.0), ("phase", None), ("request", True),
                             ("request", 1), ("seed", "42"), ("game_time", 0),
                             ("observation_contract", "wrong"),
                             *((key, "0" * 64) for key in NATIVE_INPUT_FILES)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.compare_rows([self.row | {field: value}])
        for rows in ([], [self.row, self.row]):
            with self.assertRaisesRegex(ValueError, "count mismatch"):
                self.compare_rows(rows)
        for snapshot in (None, {"pos": [0, 0]}, self.row["snapshot"] | {"snapshot_origin": "holder_latest"}):
            with self.assertRaises(ValueError):
                self.compare_rows([self.row | {"snapshot": snapshot}])

    def test_unknown_and_missing_fields_remain_explicitly_unscored(self):
        self.row["snapshot"]["future_observer_field"] = {"value": 3}
        del self.row["snapshot"]["runtime_heightmaps"]
        native, _ = self.chunk("spawn", level=False, extra={"future_extra": 7}, extra_nbt=[("FutureNativeField", 3, 9)])
        self.events[1]["snapshot"] = native
        self.refresh_identity()
        row = self.compare_rows()["requests"][0]
        paths = [field["path"] for field in row["unscored_fields"]]
        self.assertIn(["snapshot", "future_observer_field"], paths)
        self.assertIn(["nbt", "FutureNativeField"], paths)
        self.assertIn(["extra", "future_extra"], paths)
        self.assertFalse(row["runtime_heightmaps"]["scored"])
        self.assertIsNone(row["matches"])
        self.assertTrue(all(field["scored"] is False for field in row["unscored_fields"]))

    def test_tampered_capture_and_config_receipts_are_rejected(self):
        (self.root / "events.jsonl").write_text("{}\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "events.*digest mismatch"):
            self.compare_rows()
        self.write_capture()
        (self.root / "config.json").write_text("{}", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "config.*digest mismatch"):
            replay_plan(self.root)


class ReuseBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="native-reuse-", dir=ROOT / "target")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        required = REQUIRED_TOOLS | {OBSERVER + name for name in ("Cargo.toml", "Cargo.lock", "replay.rs")}
        required |= {"crates/bcore-core/Cargo.toml", "crates/bcore-worldgen/Cargo.toml", "rust-toolchain.toml"}
        self.files = {name: ("frozen " + name).encode() for name in required}
        self.files[OBSERVER + "Cargo.toml"] = b'[dependencies]\nbcore-worldgen = { path = "../../crates/bcore-worldgen" }\n'
        self.files["crates/bcore-core/Cargo.toml"] = b'[package]\nname = "bcore-core"\nversion = "0.1.0"\n'
        self.files["crates/bcore-worldgen/Cargo.toml"] = b'[dependencies]\nbcore-core = { path = "../bcore-core" }\n'
        self.tools = {name: ("invocation " + name).encode() for name in REQUIRED_TOOLS}
        write_archive(self.root / "source.zip", self.files)
        write_archive(self.root / "tools.zip", self.tools)
        self.exe = self.root / (HISTORY_PROGRAM + ".exe")
        self.exe.write_bytes(b"test-only executable identity; never executed")
        self.plan = contract_plan()
        (self.root / "replay.json").write_text(json.dumps(self.plan), encoding="utf-8")
        self.prior = {"program": HISTORY_PROGRAM, "snapshot_boundary_version": REPLAY_SCHEMA,
                      "replay_schema": REPLAY_SCHEMA, "observation_contract": OBSERVATION_CONTRACT,
                      "binary_contract": deepcopy(HISTORY_CONTRACT), "feature_trace_version": 1,
                      "build_returncode": 0, "build_command": ["cargo", "build"], "rustc": "test compiler receipt",
                      "source_sha256": hashes(self.files), "source_zip_sha256": sha(self.root / "source.zip"),
                      "tool_sha256": hashes(self.tools), "tools_zip_sha256": sha(self.root / "tools.zip"),
                      "runner_sha256": hashes(self.tools)[OBSERVER + "replay_bcore.py"],
                      "comparator_sha256": hashes(self.tools)[OBSERVER + "oracle.py"],
                      "binary_sha256": sha(self.exe), "replay_sha256": sha(self.root / "replay.json"),
                      **{key: self.plan[key] for key in NATIVE_INPUT_FILES}}
        self.write_provenance()

    def write_provenance(self):
        (self.root / "provenance.json").write_text(json.dumps(self.prior), encoding="utf-8")

    def test_reuse_preserves_build_sources_and_distinct_invocation_tool_sources(self):
        prior = validate_reuse_build(self.root, HISTORY_PROGRAM, True)
        self.assertEqual(prior["source_sha256"], hashes(self.files))
        self.assertNotEqual(prior["source_sha256"][OBSERVER + "oracle.py"], prior["tool_sha256"][OBSERVER + "oracle.py"])
        with patch("replay_bcore.subprocess.run") as run:
            validate_reuse_build(self.root, HISTORY_PROGRAM)
            run.assert_not_called()

    def test_legacy_program_version_and_unproven_build_reuse_are_rejected(self):
        original = deepcopy(self.prior)
        for field, value in (("program", "native-feature-replay"), ("snapshot_boundary_version", 2),
                             ("replay_schema", 3.0), ("observation_contract", "holder_only_v2"),
                             ("binary_contract", HISTORY_CONTRACT | {"schema": 2}),
                             ("build_returncode", 1), ("build_returncode", False), ("feature_trace_version", None)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.prior = original | {field: value}
                self.write_provenance()
                validate_reuse_build(self.root, HISTORY_PROGRAM, True)

    def test_binary_archive_manifest_plan_and_tool_metadata_tampering_is_rejected(self):
        original = deepcopy(self.prior)
        mutations = [("binary_sha256", "0" * 64), ("source_zip_sha256", "0" * 64),
                     ("replay_sha256", "0" * 64), ("tools_zip_sha256", "0" * 64),
                     ("native_events_sha256", "0" * 64), ("runner_sha256", "0" * 64),
                     ("source_sha256", original["source_sha256"] | {OBSERVER + "replay.rs": "0" * 64})]
        for field, value in mutations:
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.prior = original | {field: value}
                self.write_provenance()
                validate_reuse_build(self.root, HISTORY_PROGRAM)
        self.prior = original
        del self.prior["source_sha256"][OBSERVER + "oracle.py"]
        self.write_provenance()
        with self.assertRaisesRegex(ValueError, "required source/tool provenance"):
            validate_reuse_build(self.root, HISTORY_PROGRAM)

    def test_archive_duplicate_extra_and_wrong_member_bytes_cannot_hide_behind_zip_hash(self):
        cases = [[("a.rs", b"a"), ("a.rs", b"b")], [("a.rs", b"a"), ("extra.rs", b"a")],
                 [("a.rs", b"changed")], [("../a.rs", b"a")]]
        for index, entries in enumerate(cases):
            path = self.root / f"bad-{index}.zip"
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                with zipfile.ZipFile(path, "x") as archive:
                    for name, data in entries:
                        archive.writestr(name, data)
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                validate_archive(path, sha(path), hashes({"a.rs": b"a"}))

    def test_executable_must_advertise_exact_contract_before_replay(self):
        with patch("replay_bcore.subprocess.run", return_value=subprocess.CompletedProcess(
                [], 0, json.dumps(HISTORY_CONTRACT), "")) as run:
            self.assertEqual(validate_binary_contract(self.exe), HISTORY_CONTRACT)
            self.assertEqual(run.call_args.args[0], [str(self.exe.resolve()), "--version"])
        for field, value in (("schema", 2), ("schema", 3.0), ("observation_contract", "old")):
            with self.subTest(field=field), patch("replay_bcore.subprocess.run", return_value=subprocess.CompletedProcess(
                    [], 0, json.dumps(HISTORY_CONTRACT | {field: value}), "")), self.assertRaises(ValueError):
                validate_binary_contract(self.exe)

    def test_source_snapshot_includes_current_observer_and_python_tools(self):
        root = self.root / "workspace"
        files = self.files | {OBSERVER + "HistoryProbe.java": b"unchanged native observer",
                             "crates/bcore-registry/Cargo.toml": b'[dependencies]\nbcore-core = { path = "../bcore-core" }\n',
                             "crates/bcore-registry/src/generated.rs": b"current authenticated registry",
                             "crates/bcore-registry/data/registries_26_2.json": b'{"byte_exact": true}\r\n',
                             "crates/bcore-worldgen/src/generation/request/worker.rs": b"current production request worker",
                             "scripts/run_generation_check.py": b"current queue tool"}
        for name, data in files.items():
            file = root / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(data)
        collected = sources(root)
        for name in (OBSERVER + "oracle.py", OBSERVER + "replay_bcore.py", OBSERVER + "replay.rs",
                     OBSERVER + "HistoryProbe.java", "crates/bcore-worldgen/src/generation/request/worker.rs",
                     "crates/bcore-registry/src/generated.rs", "crates/bcore-registry/data/registries_26_2.json"):
            self.assertEqual(collected[name], files[name])
        invoked = tool_sources(root)
        self.assertTrue(REQUIRED_TOOLS <= invoked.keys())
        self.assertNotIn(OBSERVER + "replay.rs", invoked)
        self.assertEqual(invoked["scripts/run_generation_check.py"], files["scripts/run_generation_check.py"])

    def test_reuse_rejects_missing_transitive_registry_even_with_matching_archive_hashes(self):
        self.files["crates/bcore-worldgen/Cargo.toml"] += b'bcore-registry = { path = "../bcore-registry" }\n'
        (self.root / "source.zip").unlink()
        write_archive(self.root / "source.zip", self.files)
        self.prior.update(source_sha256=hashes(self.files), source_zip_sha256=sha(self.root / "source.zip"))
        self.write_provenance()
        with self.assertRaisesRegex(ValueError, "lacks path dependency crates/bcore-registry/Cargo.toml"):
            validate_reuse_build(self.root, HISTORY_PROGRAM)
        self.files["crates/bcore-registry/Cargo.toml"] = b'[dependencies]\nbcore-core = { path = "../bcore-core" }\n'
        (self.root / "source.zip").unlink()
        write_archive(self.root / "source.zip", self.files)
        self.prior.update(source_sha256=hashes(self.files), source_zip_sha256=sha(self.root / "source.zip"))
        self.write_provenance()
        self.assertEqual(validate_reuse_build(self.root, HISTORY_PROGRAM)["source_sha256"], hashes(self.files))

    def test_target_specific_dependency_cannot_escape_the_source_archive(self):
        for index, dependency in enumerate(("../../../outside", "/absolute", "C:/outside")):
            files = self.files | {OBSERVER + "Cargo.toml": (
                '[target.\'cfg(windows)\'.dependencies]\nexternal = { path = "' + dependency + '" }\n').encode()}
            path = self.root / f"external-dependency-{index}.zip"
            write_archive(path, files)
            with self.subTest(dependency=dependency), self.assertRaisesRegex(ValueError, "path dependency"):
                validate_source_dependencies(path)


class NativeHistoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="native-history-test-", dir=ROOT / "target")
        cls.root = Path(cls.temp.name)
        cls.origin = cls.root / "origin"
        cls.manifest = extract_fixture("origin-heightmaps-26.1.zip", cls.origin)
        cls.events = load_events(cls.origin)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def test_real_engine_stages_run_and_full_does_not_imply_gameplay_ticks(self):
        complete = self.events[-1]
        self.assertEqual(complete["event"], "oracle_complete")
        self.assertEqual((complete["gameplay_ticks"], complete["game_time"]), (0, 0))
        starts = {e["seq"]: e for e in self.events if e["event"] == "stage_enter"}
        ends = {e["stage_id"]: e for e in self.events if e["event"] == "stage_exit"}
        self.assertEqual(starts.keys(), ends.keys())
        counts = Counter(e["status"] for e in starts.values())
        self.assertEqual(counts["minecraft:features"], 9)
        self.assertEqual(counts["minecraft:full"], 1)
        self.assertEqual(counts["minecraft:spawn"], 1)
        self.assertTrue(any(e["event"] == "task_create" for e in self.events))
        self.assertTrue(any(e["event"] == "holder_claim" and e["acquired"] for e in self.events))
        self.assertTrue(any(e["event"] == "world_tick_request" for e in self.events))
        feature_snapshots = [e for e in self.events if e["event"] == "stage_snapshot" and e["status"] == "minecraft:features"]
        self.assertEqual(len(feature_snapshots), 18)
        self.assertTrue(all(len(e["chunks"]) == 289 for e in feature_snapshots))

    def test_native_wg_getter_differs_from_live_predicate_after_decoration(self):
        observations = [e for e in self.events if e["event"] == "world_height_query"
                        and e["source_sequence"] == 1 and "patch_grass_forest" in e["decoration_label"]]
        self.assertEqual([(e["xz"], e["returned"], e["live_predicate_first_free"]) for e in observations],
                         [([15, 6], 126, 133), ([6, 5], 127, 128)])

    def test_complete_chunk_decode_is_cross_checked_against_native_section_getters(self):
        registry = {state_key(s): s["id"] for s in json.loads((self.origin / "block-states.json").read_text())}
        requests = [e for e in self.events if e["event"] == "request_exit"]
        for event in requests:
            decoded = decode_snapshot(self.origin, event["snapshot"], registry)
            self.assertEqual(len(decoded["states"]), 98304)
            self.assertEqual(len(decoded["biomes"]), 1536)
        self.assertNotIn("WORLD_SURFACE_WG", decoded["extra"]["heightmaps_live"])
        self.assertEqual(decoded["extra"]["chunk_class"], "net.minecraft.world.level.chunk.LevelChunk")
        native = decode_snapshot(self.origin, requests[4]["snapshot"], registry)
        self.assertIn("WORLD_SURFACE_WG", native["extra"]["heightmaps_live"])
        self.assertEqual(native["states"][(126 + 64) * 256 + 10 * 16 + 14], 2248)
        self.assertEqual(sha(self.origin / "events.jsonl"), json.loads((self.origin / "capture.json").read_text())["events_sha256"])

    def test_native_adjacent_reversal_changes_real_blocks_not_just_metadata(self):
        direct, reverse = self.root / "direct", self.root / "reverse"
        extract_fixture("adjacent-26.1.zip", direct)
        extract_fixture("reverse-26.1.zip", reverse)
        diff = native_diff(direct, reverse)
        self.assertTrue(diff["same_source_set"])
        self.assertFalse(diff["same_requests"])
        origin = next(c for c in diff["chunks"] if c["pos"] == (0, 0))
        self.assertEqual(origin["states"]["mismatches"], 1129)
        self.assertEqual(origin["states"]["first"]["pos"], [2, -52, 0])
        self.assertEqual((origin["states"]["first"]["expected"], origin["states"]["first"]["actual"]), (25170, 27924))

    def test_replay_plan_keeps_request_order_and_native_provenance(self):
        plan = replay_plan(self.origin)
        self.assertEqual(plan["bootstrap_requests"], [])
        self.assertEqual([r["status"] for r in plan["requests"]], ["biomes", "noise", "surface", "carvers", "features", "full"])
        self.assertEqual(plan["seed"], "846692123413862008")
        self.assertEqual(plan["native_provenance_sha256"], sha(self.origin / "provenance.json"))
        self.assertEqual(plan["schema"], REPLAY_SCHEMA)
        self.assertEqual(plan["observation_contract"], OBSERVATION_CONTRACT)
        self.assertEqual(plan["native_config_sha256"], sha(self.origin / "config.json"))
        self.assertEqual(plan["native_registry_sha256"], sha(self.origin / "block-states.json"))
        self.assertEqual(plan["game_time"], "0")

    def test_replay_rejects_unknown_or_nonzero_observation_clock(self):
        for replacement in (None, 1, False):
            events = [event | {"game_time": replacement} if event["event"] == "request_exit" else event
                      for event in self.events]
            with self.subTest(clock=replacement), patch("oracle.load_events", return_value=events):
                with self.assertRaisesRegex(ValueError, "game time"):
                    replay_plan(self.origin)

    def test_verified_summary_keeps_inflight_stages_separate_from_completed_work(self):
        # Build a terminal prefix from real native observations in an isolated
        # extraction. The immutable source fixture is never rewritten.
        capture = self.root / "inflight-prefix"
        extract_fixture("origin-heightmaps-26.1.zip", capture)
        full = next(e for e in self.events if e["event"] == "stage_enter" and e["status"] == "minecraft:full")
        events = [e for e in self.events if e["seq"] <= full["seq"]]
        finished = {e["stage_id"] for e in events if e["event"] == "stage_exit"}
        pending = [{k: v for k, v in e.items() if k in ("pos", "status", "request", "thread")} | {"stage_id": e["seq"]}
                   for e in events if e["event"] == "stage_enter" and e["seq"] not in finished]
        events.append(self.events[-1] | {"seq": len(events) + 1, "inflight_stages": pending})
        (capture / "events.jsonl").write_text("".join(json.dumps(e) + "\n" for e in events), encoding="utf-8")
        metadata = json.loads((capture / "capture.json").read_text())
        metadata["events_sha256"] = sha(capture / "events.jsonl")
        (capture / "capture.json").write_text(json.dumps(metadata), encoding="utf-8")
        ordinary, verified = summarize(capture), summarize(capture, verify=True)
        self.assertGreater(verified["verified_blobs"], 0)
        self.assertEqual(verified["inflight_stages"], pending)
        self.assertEqual(verified["completed_stages"], ordinary["completed_stages"])
        self.assertEqual(verified["stages"]["minecraft:full"], 1)
        self.assertEqual(verified["completed_stages"].get("minecraft:full", 0), 0)

    def test_feature_capture_contains_native_rng_continuation_and_cross_chunk_writes(self):
        capture = self.root / "forest-grass"
        extract_fixture("forest-grass-26.1.zip", capture)
        events = load_events(capture)
        snapshots = [e for e in events if e["event"] == "feature_snapshot"]
        self.assertEqual([e["boundary"] for e in snapshots], ["before", "after"])
        self.assertTrue(all(len(e["chunks"]) == 9 for e in snapshots))
        by_id = {e["seq"]: e for e in events}
        entry, exit_event = [by_id[e["feature_event"]] for e in snapshots]
        self.assertEqual(exit_event["rng"]["next_i64_from_copy"], -2473347630682788069)
        writes = [[*e["pos"], e["state"], e["flags"]] for e in events
                  if entry["seq"] < e["seq"] < exit_event["seq"] and e["event"] == "world_set_block" and e["accepted"]]
        self.assertEqual(writes, [[14, 126, 10, 2248, 2], [8, 127, -1, 2248, 2],
                                  [20, 125, 1, 2248, 2], [13, 127, 2, 2248, 2], [6, 127, 6, 2248, 2]])


if __name__ == "__main__":
    unittest.main()
