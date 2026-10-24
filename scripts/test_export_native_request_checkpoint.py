"""Synthetic receipt/aggregation tests. No native execution or native rescoring."""
from copy import deepcopy
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import export_native_request_checkpoint as checkpoint

oracle = checkpoint.oracle
runner = checkpoint.replay_bcore


def write(path, value):
    path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")


def comparison(total=None, differences=()):
    differences = list(differences)
    result = {"scored": True, "matches": not differences, "mismatches": len(differences),
              "differences": differences, "examples": differences[:16]}
    if total is not None:
        result.update(total=total, first=differences[0] if differences else None, pairs=[])
    return result


class ExportNativeRequestTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="native-export-test-", dir=checkpoint.ROOT / "target")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        for directory in ("target/generation-checks", "docs/metrics", "scripts"):
            (self.root / directory).mkdir(parents=True)
        exporter_file = self.root / "scripts/export_native_request_checkpoint.py"
        exporter_file.write_bytes(Path(checkpoint.__file__).read_bytes())
        for target, value in (("ROOT", self.root), ("__file__", str(exporter_file))):
            mocked = patch.object(checkpoint, target, value)
            mocked.start()
            self.addCleanup(mocked.stop)
        mocked = patch.object(runner.subprocess, "run", side_effect=AssertionError("must not execute a binary"))
        mocked.start()
        self.addCleanup(mocked.stop)
        for target in ("compare", "decode_snapshot", "compare_snapshot"):
            mocked = patch.object(oracle, target, side_effect=AssertionError("must not rescore native output"))
            mocked.start()
            self.addCleanup(mocked.stop)
        required = runner.REQUIRED_TOOLS | {runner.OBSERVER + name for name in ("Cargo.toml", "Cargo.lock", "replay.rs")}
        required |= {"crates/bcore-core/Cargo.toml", "crates/bcore-worldgen/Cargo.toml", "rust-toolchain.toml"}
        self.sources = {name: ("synthetic frozen source: " + name).encode() for name in required}
        self.sources[runner.OBSERVER + "Cargo.toml"] = b'[dependencies]\nbcore-worldgen = { path = "../../crates/bcore-worldgen" }\n'
        self.sources["crates/bcore-core/Cargo.toml"] = b'[package]\nname = "bcore-core"\nversion = "0.1.0"\n'
        self.sources["crates/bcore-worldgen/Cargo.toml"] = b'[dependencies]\nbcore-core = { path = "../bcore-core" }\n'
        self.tools = {name: ("synthetic invocation tool: " + name).encode() for name in runner.REQUIRED_TOOLS}
        self.test_job = self.make_tests()

    def queue_job(self, name, command, log):
        path = self.root / "target/generation-checks" / (name + ".json")
        write(path, {"job": name, "state": "finished", "exit_code": 0, "workdir": str(self.root),
                     "command": command, "started": 10, "execution_started": 13, "finished": 23})
        path.with_suffix(".log").write_text(log, encoding="utf-8")
        return path

    def make_tests(self):
        lines = ["Finished `release` profile [optimized] target(s) in 2.50s"]
        for name, passed, ignored, seconds in (("first", 2, 1, 1.25), ("second", 5, 0, 0.75)):
            lines += [f"Running unittests {name}.exe", json.dumps({"type": "suite", "event": "started", "test_count": passed + ignored})]
            lines += [json.dumps({"type": "test", "event": "ok", "name": f"{name}_{i}", "exec_time": 1}) for i in range(passed)]
            lines += [json.dumps({"type": "test", "event": "ignored", "name": "manual"}) for _ in range(ignored)]
            lines += [json.dumps({"type": "suite", "event": "ok", "passed": passed, "failed": 0,
                                  "ignored": ignored, "measured": 0, "filtered_out": 0, "exec_time": seconds})]
        command = ["cargo", "+nightly", "test", "--workspace", "--release", "--locked", "--no-fail-fast", "-j", "1",
                   "--", "-Z", "unstable-options", "--format=json", "--report-time", "--test-threads=2"]
        return self.queue_job("workspace-tests", command, "\n".join(lines) + "\n")

    def make_history(self, name="first", reuse=None, binary=b"synthetic executable identity; never executed", sources=None):
        capture = self.root / "target" / (name + "-native")
        out = self.root / "target" / (name + "-replay")
        (capture / "blobs").mkdir(parents=True)
        out.mkdir()

        def blob(data, kind):
            name = hashlib.sha256(data).hexdigest() + "." + kind + ".gz"
            (capture / "blobs" / name).write_bytes(gzip.compress(data, mtime=0))
            return name

        pos, absent_pos = [3, -4], [17, 9]
        native = {"pos": pos, "status": "minecraft:noise", "nbt": blob(b"synthetic opaque NBT", "nbt"),
                  "extra": blob(b"synthetic opaque extras", "json")}
        absent = {"pos": absent_pos, "absent": True}
        config = {"seed": "-9223372036854775807", "bootstrap": "before_spawn", "watch": [pos, absent_pos],
                  "requests": [{"pos": pos, "status": status} for status in ("noise", "full")]}
        events = [{"seq": 1, "event": "bootstrap_complete", "mode": "before_spawn", "game_time": 0,
                   "native_seed": int(config["seed"])}]
        events += [{"seq": i + 2, "event": "request_exit", "phase": "requests", "request": i, "spec": spec,
                    "snapshot": native, "watch": [native, absent], "game_time": 0} for i, spec in enumerate(config["requests"])]
        events += [{"seq": 4, "event": "oracle_complete", "game_time": 0, "gameplay_ticks": 0}]
        write(capture / "config.json", config)
        (capture / "events.jsonl").write_text("".join(json.dumps(e) + "\n" for e in events), encoding="utf-8")
        write(capture / "block-states.json", [{"id": 0, "Name": "minecraft:air"}, {"id": 1, "Name": "minecraft:stone"}])
        write(capture / "provenance.json", {"config_sha256": oracle.sha(capture / "config.json")})
        write(capture / "capture.json", {"events_sha256": oracle.sha(capture / "events.jsonl"),
                                         "provenance_sha256": oracle.sha(capture / "provenance.json")})
        identity = oracle.native_input_identity(capture)
        plan = oracle.plan_from_events(config, events, identity)
        write(out / "replay.json", plan)
        chunk = {"pos": pos, "states": [0, 0, 0], "biomes": ["minecraft:plains"], "min_y": 0,
                 "raw_object_kind": "Proto", "snapshot_boundary": "proto_save",
                 "chunk_class": "net.minecraft.world.level.chunk.ProtoChunk",
                 "persisted_status": "minecraft:noise", "serialized_status": "minecraft:noise",
                 "block_entities": [], "saved_ticks": {"block_ticks": [], "fluid_ticks": []},
                 "worldgen_heightmaps": {"WORLD_SURFACE_WG": [0, 1]}, "runtime_heightmaps": None,
                 "light": {"min_section_y": -1, "sections": [{"sky": [0, 0], "block": None}]}}
        stages = [{"status": "minecraft:" + status, "state": "Partial" if status == "full" else "Complete",
                   "attempts": 1, "missing": ["world registration"] if status == "full" else []} for status in oracle.STATUSES]
        raw = [{k: plan[k] for k in ("schema", "observation_contract", "seed", "game_time", *identity)} | {
            "request": i, "phase": "requests", "spec": spec, "complete": False, "coverage_status": "minecraft:full",
            "stages": deepcopy(stages), "feature_sources": [], "snapshot": chunk | {"snapshot_origin": "returned_object"},
            "watch": [deepcopy(chunk) | {"snapshot_origin": "holder_latest"}, absent | {"snapshot_origin": "holder_latest"}]}
            for i, spec in enumerate(plan["requests"])]
        summary = {"pos": pos, "snapshot_origin": "returned_object", "presence": comparison(), "native_nbt": native["nbt"],
                   "native_extra": native["extra"], "scored_matches": True, "matches": None, "scope": "scored snapshot fields",
                   "unscored_fields": [{"side": "bcore", "path": ["snapshot", "raw_object_kind"], "scored": False,
                                        "reason": "no cross-engine comparison for this field"}],
                   "states": comparison(3), "biomes": comparison(1), "geometry": comparison(),
                   "generated_metadata": {key: comparison() for key in checkpoint.METADATA},
                   "native_effect_counts": {key: 0 for key in ("block_entities", "entities", "block_ticks", "fluid_ticks")},
                   "serialized_light": {"scored": True, "matches": True, "native_is_light_on": False, "bcore_snapshot_available": True,
                                        "geometry_matches": True, "geometry": comparison(), "presence_mismatches": 0,
                                        "length_mismatches": 0, "byte_mismatches": 0, "layers": [
                                            {"y": -1, "kind": "sky", "expected_present": True, "actual_present": True,
                                             "presence_matches": True, "bytes": comparison(2)},
                                            {"y": -1, "kind": "block", "expected_present": False, "actual_present": False, "presence_matches": True}]}}
        for group, names in checkpoint.MAPS.items():
            maps = {}
            for map_name in names:
                present = map_name == "WORLD_SURFACE_WG"
                maps[map_name] = {"scored": True, "matches": True, "native_present": present, "bcore_present": present, "presence_matches": True}
                if present:
                    maps[map_name]["values"] = comparison(2)
            summary[group] = maps if group == "worldgen_heightmaps" else {"scored": True, "matches": True, "maps": maps}
        absent_summary = {"pos": absent_pos, "snapshot_origin": "holder_latest", "presence": comparison(), "native_nbt": None,
                          "native_extra": None, "scored_matches": True, "matches": True, "scope": "presence", "unscored_fields": [],
                          "content": {"scored": False, "reason": "one or both chunks explicitly absent"}}
        rows = [deepcopy(summary) | {"request": r["request"], "spec": r["spec"], "native_event": r["request"] + 2,
                 "watch": [deepcopy(summary) | {"snapshot_origin": "holder_latest"}, deepcopy(absent_summary)],
                 "source_order": comparison(0), "requested_scored_matches": True,
                 **{"bcore_" + k: r[k] for k in ("complete", "coverage_status", "stages", "feature_sources")},
                 "unscored_row_fields": [{"path": [key], "scored": False, "reason": "BCore diagnostic"}
                                          for key in ("complete", "coverage_status", "stages", "feature_sources")]}
                for r in raw]
        diff = {"schema": 3, "observation_contract": oracle.OBSERVATION_CONTRACT, "capture": str(capture), **identity,
                "comparator_sha256": runner.hashes(self.tools)[runner.OBSERVER + "oracle.py"], "requests": rows,
                "scored_matches": True, "matches": None, "limitations": ["synthetic fixtures exercise receipt contracts only"]}
        files = sources or self.sources
        if reuse:
            (out / "source.zip").write_bytes((reuse["out"] / "source.zip").read_bytes())
        else:
            runner.write_archive(out / "source.zip", files)
        runner.write_archive(out / "tools.zip", self.tools)
        exe = out / (runner.HISTORY_PROGRAM + ".exe")
        exe.write_bytes(binary)
        prior = {"program": runner.HISTORY_PROGRAM, "snapshot_boundary_version": 3, "replay_schema": 3,
                 "observation_contract": oracle.OBSERVATION_CONTRACT, "binary_contract": runner.HISTORY_CONTRACT,
                 "feature_trace_version": 1, "build_returncode": 0, "build_command": ["cargo", "build", "--release"],
                 "rustc": "synthetic compiler receipt", "source_sha256": runner.hashes(files),
                 "source_zip_sha256": oracle.sha(out / "source.zip"), "binary_sha256": oracle.sha(exe),
                 "tool_sha256": runner.hashes(self.tools), "tools_zip_sha256": oracle.sha(out / "tools.zip"),
                 "runner_sha256": runner.hashes(self.tools)[runner.OBSERVER + "replay_bcore.py"],
                 "comparator_sha256": diff["comparator_sha256"], "replay_sha256": oracle.sha(out / "replay.json"), **identity,
                 "run_command": [str(exe), str(out / "replay.json"), str(out / "chunks.jsonl")], "rayon_num_threads": 1}
        command = ["python", "-B", str(self.root / runner.OBSERVER / "replay_bcore.py"), str(capture), "--output", str(out)]
        if reuse:
            prior.update(reused_build=str(reuse["out"]), reused_provenance_sha256=oracle.sha(reuse["out"] / "provenance.json"))
            command += ["--reuse-build", str(reuse["out"])]
        job = self.queue_job("replay-" + name, command, "")
        (out / "run.log").write_text("synthetic run receipt\n", encoding="utf-8")
        history = dict(out=out, capture=capture, raw=raw, diff=diff, prior=prior, job=job)
        self.refresh(history)
        return history

    def refresh(self, history):
        out, diff, prior = history["out"], history["diff"], history["prior"]
        (out / "chunks.jsonl").write_text("".join(json.dumps(row) + "\n" for row in history["raw"]), encoding="utf-8")
        prior["result_sha256"] = diff["rust_output_sha256"] = oracle.sha(out / "chunks.jsonl")
        write(out / "provenance.json", prior)
        write(out / "comparison.json", diff)
        write(history["job"].with_suffix(".log"), {"output": str(out), "requests": runner.comparison_summary(diff)})

    def publish(self, histories, name="checkpoint.json"):
        return checkpoint.export(self.test_job, [h["job"] for h in histories], self.root / "docs/metrics" / name)

    def test_json_suite_totals_hashes_and_compiler_timing_are_preserved(self):
        result = checkpoint.tests(self.test_job)
        self.assertEqual(result["totals"], dict(passed=7, failed=0, ignored=1, measured=0, filtered_out=0))
        self.assertEqual(result["target_count"], 2)
        self.assertEqual(result["per_test_timings"], 7)
        self.assertEqual(result["build_seconds"], [2.5])
        self.assertEqual(result["harness_seconds"], 2)
        self.assertEqual(result["queue_wait_seconds"], 3)
        self.assertEqual(result["execution_seconds"], 10)
        self.assertEqual(result["unattributed_seconds"], 5.5)
        self.assertEqual(result["status"]["sha256"], oracle.sha(self.test_job))
        self.assertEqual(result["log"]["sha256"], oracle.sha(self.test_job.with_suffix(".log")))
        self.assertEqual(result["command"], checkpoint.read(self.test_job)["command"])

    def test_failed_missing_and_nonroot_queue_receipts_are_rejected(self):
        original = checkpoint.read(self.test_job)
        for changed in ({"state": "waiting"}, {"state": "failed"}, {"exit_code": 1}, {"exit_code": False},
                        {"exit_code": None}, {"finished": None}, {"execution_started": 99}, {"job": "different"},
                        {"workdir": str(self.root / "target")}):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                write(self.test_job, original | changed)
                checkpoint.tests(self.test_job)
        write(self.test_job, original)
        self.test_job.with_suffix(".log").unlink()
        with self.assertRaises(FileNotFoundError):
            checkpoint.tests(self.test_job)
        with self.assertRaises(FileNotFoundError):
            checkpoint.tests(self.test_job.with_name("missing.json"))

    def test_missing_or_failed_harness_results_cannot_use_exit_zero(self):
        log = self.test_job.with_suffix(".log")
        original = log.read_text(encoding="utf-8")
        for text in ("", original + "Running unittests unfinished.exe\n",
                     "\n".join(original.splitlines()[:-1]),
                     original.replace('"event": "ok", "passed": 5', '"event": "failed", "passed": 5'),
                     original.replace('"test_count": 5', '"test_count": 6')):
            with self.subTest(text=text[-80:]), self.assertRaises(ValueError):
                log.write_text(text, encoding="utf-8")
                checkpoint.tests(self.test_job)

    def test_actual_commands_reject_wrappers_filters_and_other_programs(self):
        for command in (["python", "-c", "cargo test --workspace --release"],
                        ["cargo", "test", "--workspace", "--release", "--no-run"],
                        ["cargo", "test", "--release", "--", "--workspace"],
                        ["cargo", "test", "--workspace", "--release", "--", "some_filter"],
                        ["cargo", "build", "--workspace", "--release"]):
            with self.subTest(command=command), self.assertRaises(ValueError):
                checkpoint.test_command(command)
        script = str(self.root / runner.OBSERVER / "replay_bcore.py")
        for command in (["python", "-c", script, "capture", "--output", "target/replay"],
                        ["python", "elsewhere/replay_bcore.py", "capture", "--output", "target/replay"],
                        ["python", script, "capture", "--output", "target/replay", "--feature-case", "case.json"]):
            with self.subTest(command=command), self.assertRaises(ValueError):
                checkpoint.replay_command(command)

    def test_watch_mismatches_keep_requested_and_watch_counts_separate(self):
        history = self.make_history()
        row = history["diff"]["requests"][0]
        watch = row["watch"][0]
        watch["states"] = comparison(3, [{"index": 1, "expected": 1, "actual": 0}])
        watch["generated_metadata"]["chunk_lifecycle"] = comparison(differences=[{
            "path": ["serialized_status"], "expected": "minecraft:light", "actual": "minecraft:noise"}])
        watch["scored_matches"], watch["matches"] = False, False
        row["scored_matches"], row["matches"] = False, False
        history["diff"].update(scored_matches=False, matches=False)
        self.refresh(history)
        result = self.publish([history])
        totals = result["totals"]
        self.assertEqual(totals["requests"], 2)
        self.assertEqual(totals["requested"]["snapshots"], 2)
        self.assertEqual(totals["watch"]["snapshots"], 4)
        self.assertEqual(totals["requested"]["fields"]["states"]["observations"], 6)
        self.assertEqual(totals["watch"]["fields"]["states"]["observations"], 6)
        self.assertEqual(totals["requested"]["fields"]["states"]["differences"], 0)
        self.assertEqual(totals["watch"]["fields"]["states"]["differences"], 1)
        self.assertEqual(totals["watch"]["fields"]["states"]["unscored"], 2)
        self.assertEqual(totals["watch"]["fields"]["generated_metadata/chunk_lifecycle"]["differences"], 1)
        self.assertTrue(result["histories"][0]["requests"][0]["requested"]["scored_matches"])
        self.assertFalse(result["scored_matches"])
        self.assertFalse(result["matches"])
        self.assertNotIn("distinct_chunks", totals)
        self.assertEqual(totals, {"histories": 1, "bootstrap_calls": 0, **result["histories"][0]["totals"]})

    def test_unknown_fields_unavailable_metadata_and_partial_stages_are_not_complete(self):
        history = self.make_history()
        for row in history["diff"]["requests"]:
            row["generated_metadata"]["save_boundary"] = {"scored": False, "reason": "unknown native serialization class"}
            row["unscored_fields"].append({"path": ["future_native_field"], "side": "native", "scored": False, "reason": "unsupported"})
            row["generated_metadata"]["future_comparison"] = {"scored": True, "matches": True, "opaque": 19}
            for value in row["runtime_heightmaps"]["maps"].values():
                value.clear()
                value.update(scored=False, reason="heightmap observation missing")
            row["runtime_heightmaps"].update(scored=False, matches=None)
        self.refresh(history)
        result = self.publish([history])
        self.assertTrue(result["scored_matches"])
        self.assertIsNone(result["matches"])
        self.assertEqual(result["totals"]["bcore"]["partial_requests"], 2)
        self.assertEqual(result["totals"]["bcore"]["complete_requests"], 0)
        value = result["totals"]["requested"]["fields"]["generated_metadata/save_boundary"]
        self.assertEqual((value["count"], value["scored"], value["unscored"], value["matches"]), (2, 0, 2, 0))
        self.assertIsNone(value["differences"])
        unknown = result["histories"][0]["requests"][0]["requested"]["unscored_fields"]
        self.assertTrue(all(item["scored"] is False for item in unknown))
        self.assertIn(["future_native_field"], [item["path"] for item in unknown])
        self.assertEqual(next(item["value"]["opaque"] for item in unknown if "value" in item), 19)

    def test_height_light_metadata_and_source_order_differences_keep_their_scopes(self):
        history = self.make_history()
        row = history["diff"]["requests"][0]
        watch = row["watch"][0]
        height = watch["worldgen_heightmaps"]["WORLD_SURFACE_WG"]
        height.update(matches=False, values=comparison(2, [{"index": 1, "expected": 0, "actual": 1}]))
        watch["worldgen_heightmaps"]["OCEAN_FLOOR_WG"].update(native_present=True, presence_matches=False, matches=False)
        light = watch["serialized_light"]
        light["layers"][0]["bytes"] = comparison(3, [{"index": 2, "expected_present": True, "actual_present": False}]) | {
            "expected_length": 3, "actual_length": 2}
        light["layers"][1].update(expected_present=True, presence_matches=False)
        light.update(matches=False, presence_mismatches=1, length_mismatches=1, byte_mismatches=1)
        watch["generated_metadata"]["structures"] = comparison(differences=[{"path": ["starts", "one"]}, {"path": ["starts", "two"]}])
        watch["generated_metadata"]["save_boundary"] = comparison(differences=[{"path": [], "expected": "level_save", "actual": "proto_save"}])
        sources = [{"sequence": 1, "source": [99, -77]}]
        history["raw"][0]["feature_sources"] = row["bcore_feature_sources"] = sources
        row["source_order"] = comparison(1, [{"index": 0, "expected_present": False, "actual": [99, -77]}]) | {
            "expected_length": 0, "actual_length": 1}
        watch.update(scored_matches=False, matches=False)
        row.update(scored_matches=False, matches=False)
        history["diff"].update(scored_matches=False, matches=False)
        self.refresh(history)
        result = self.publish([history])
        totals = result["totals"]
        fields = totals["watch"]["fields"]
        for name in ("worldgen_heightmaps/WORLD_SURFACE_WG/values", "worldgen_heightmaps/OCEAN_FLOOR_WG/presence",
                     "serialized_light/presence", "serialized_light/length", "serialized_light/bytes", "generated_metadata/save_boundary"):
            self.assertEqual(fields[name]["differences"], 1, name)
            self.assertEqual(totals["requested"]["fields"][name]["differences"], 0, name)
        self.assertEqual(fields["serialized_light/bytes"]["observations"], 5)
        self.assertEqual(fields["generated_metadata/structures"]["differences"], 2)
        self.assertEqual(fields["generated_metadata/structures"]["count"], 4)
        self.assertEqual(fields["generated_metadata/structures"]["scored"], 2)
        self.assertEqual(fields["generated_metadata/structures"]["matches"], 1)
        self.assertEqual(totals["source_order"]["differences"], 1)
        boundary = result["histories"][0]["requests"][0]["watch"][0]["fields"]["generated_metadata/save_boundary"]
        self.assertEqual(boundary["differences"][0]["expected"], "level_save")

    def test_explicit_missing_return_is_presence_only_not_zero_content_differences(self):
        history = self.make_history()
        raw, row = history["raw"][0], history["diff"]["requests"][0]
        raw.update(complete=None, coverage_status=None, stages=None)
        raw["snapshot"] = {"pos": raw["spec"]["pos"], "snapshot_origin": "returned_object", "absent": True}
        for key in ("states", "biomes", "geometry", "worldgen_heightmaps", "runtime_heightmaps", "serialized_light", "generated_metadata", "native_effect_counts"):
            del row[key]
        row.update(bcore_complete=None, bcore_coverage_status=None, bcore_stages=None,
                   presence=comparison(differences=[{"path": [], "expected": True, "actual": False}]),
                   scope="presence", content={"scored": False, "reason": "one or both chunks explicitly absent"},
                   requested_scored_matches=False, scored_matches=False, matches=False)
        history["diff"].update(scored_matches=False, matches=False)
        self.refresh(history)
        result = self.publish([history])
        requested = result["totals"]["requested"]
        self.assertEqual((requested["native_present"], requested["bcore_present"]), (2, 1))
        self.assertEqual(requested["fields"]["presence"]["differences"], 1)
        self.assertEqual(requested["fields"]["states"]["observations"], 3)
        self.assertEqual(requested["fields"]["states"]["unscored"], 1)
        self.assertEqual(result["totals"]["bcore"]["absent_requests"], 1)

    def test_reuse_accepts_invocation_tools_distinct_from_build_sources(self):
        first = self.make_history()
        self.tools = {name: data + b" changed invocation" for name, data in self.tools.items()}
        second = self.make_history("second", reuse=first)
        result = self.publish([first, second])
        self.assertEqual(result["totals"]["histories"], 2)
        self.assertNotEqual(first["prior"]["source_sha256"][runner.OBSERVER + "oracle.py"], second["diff"]["comparator_sha256"])
        self.assertEqual(len(result["histories"][1]["evidence"]["build_chain"]), 2)
        self.assertEqual(result["histories"][1]["comparator_sha256"], second["prior"]["comparator_sha256"])

    def test_mixed_binary_or_sources_cannot_form_one_checkpoint(self):
        first = self.make_history()
        second = self.make_history("binary", binary=b"different executable")
        third = self.make_history("source", sources=self.sources | {runner.OBSERVER + "replay.rs": b"different source"})
        for other in (second, third):
            with self.subTest(history=other["out"]), self.assertRaisesRegex(ValueError, "mixed source/binary"):
                self.publish([first, other])
        self.assertFalse((self.root / "docs/metrics/checkpoint.json").exists())

    def test_provenance_links_command_and_schema_corruption_fail_closed(self):
        history = self.make_history()
        original = deepcopy(history)
        cases = ("schema2", "raw_schema2", "comparator", "result_hash", "run_command", "missing_watch", "missing_row", "coverage", "erased_watch")
        for case in cases:
            history = deepcopy(original)
            row = history["diff"]["requests"][0]
            if case == "schema2":
                history["diff"]["schema"] = 2
            elif case == "raw_schema2":
                history["raw"][0]["schema"] = 2
            elif case == "comparator":
                history["diff"]["comparator_sha256"] = "0" * 64
            elif case == "run_command":
                history["prior"]["run_command"][-1] = str(self.root / "target/other.jsonl")
            elif case == "missing_watch":
                row["watch"].pop()
            elif case == "missing_row":
                history["diff"]["requests"].pop()
            elif case == "coverage":
                row["bcore_complete"] = True
            elif case == "erased_watch":
                row["watch"][0]["states"] = comparison(3, [{"expected": 1, "actual": 0}])
            self.refresh(history)
            if case == "result_hash":
                prior = checkpoint.read(history["out"] / "provenance.json")
                prior["result_sha256"] = "0" * 64
                write(history["out"] / "provenance.json", prior)
            with self.subTest(case=case), self.assertRaises((ValueError, KeyError)):
                self.publish([history])
            self.assertFalse((self.root / "docs/metrics/checkpoint.json").exists())
        self.refresh(original)
        (history["capture"] / "events.jsonl").write_text("{}\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "events.*digest mismatch"):
            self.publish([original])

    def test_replay_needs_successful_job_and_its_own_logged_output(self):
        history = self.make_history()
        status = checkpoint.read(history["job"])
        write(history["job"], status | {"exit_code": 1})
        with self.assertRaisesRegex(ValueError, "exit-zero"):
            self.publish([history])
        write(history["job"], status)
        logged = checkpoint.read(history["job"].with_suffix(".log"))
        write(history["job"].with_suffix(".log"), logged | {"output": str(self.root / "target/some-other-run")})
        with self.assertRaisesRegex(ValueError, "job log"):
            self.publish([history])

    def test_output_is_deterministic_create_new_and_explicit_history_only(self):
        first = self.make_history()
        second = self.make_history("second", reuse=first)
        self.publish([first, second], "one.json")
        self.publish([second, first], "two.json")
        one = self.root / "docs/metrics/one.json"
        saved = one.read_bytes()
        self.assertEqual(saved, (one.parent / "two.json").read_bytes())
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.publish([first], "one.json")
        self.assertEqual(one.read_bytes(), saved)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.publish([first, first], "duplicate.json")
        with self.assertRaisesRegex(ValueError, "at least one"):
            self.publish([], "empty.json")
        with self.assertRaisesRegex(ValueError, "docs/metrics"):
            checkpoint.export(self.test_job, [first["job"]], self.root / "target/export.json")


if __name__ == "__main__":
    unittest.main()
