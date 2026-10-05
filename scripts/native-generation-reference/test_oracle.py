"""Portable actual-engine evidence and corruption/history rejection checks."""

from collections import Counter
import hashlib
import io
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile

from oracle import NbtReader, compare_generated_metadata, compare_serialized_light, decode_snapshot, load_events, native_diff, nested_mismatch, replay_plan, sha, state_key, summarize, unpack, validate_stages

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
