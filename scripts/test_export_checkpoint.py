"""Check publication boundaries using synthetic replay histories in a temporary root."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import export_checkpoint


class CheckpointPublicationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(dir=export_checkpoint.ROOT / "target")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.metrics = self.root / "docs/metrics"
        self.metrics.mkdir(parents=True)
        self.wave = self.root / "wave"
        self.wave.mkdir()
        self.tests = self.root / "tests.json"
        self.write(self.tests, {"state": "finished", "exit_code": 0, "job": "synthetic-workspace",
                                "command": ["cargo", "test", "--workspace"]})
        self.tests.with_suffix(".log").write_text(
            "test skipped_case ... ignored\n"
            "test result: ok. 7 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n"
            "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            encoding="utf-8",
        )
        self.histories = {
            "river": self.history("river", [self.request([0, 0], 3), self.request([1, 0], 0)]),
            "ridge": self.history("ridge", [self.request([-1, 2], 0)]),
        }
        self.write(self.wave / "status.json", {"state": "finished", "cases": [
            {"capture": name, "exit_code": 0} for name in self.histories
        ]})
        self.before = copy.deepcopy(self.histories["river"])
        self.before["requests"][0]["states"]["mismatches"] = 17
        self.before["comparison_sha256"] = "b" * 64
        self.carried = self.metrics / "published-history.json"
        historical = copy.deepcopy(list(self.histories.values()))
        historical[0]["comparison_sha256"] = "c" * 64
        self.carried_record = {
            "histories": historical,
            "before_after": [{"history": "river", "requests": 2,
                              "before_state_differences": 17, "after_state_differences": 3,
                              "before_comparison_sha256": "b" * 64,
                              "after_comparison_sha256": "c" * 64}],
        }
        self.write(self.carried, self.carried_record)
        self.output = self.metrics / "new-checkpoint.json"
        self.enterContext(patch.object(export_checkpoint, "ROOT", self.root))
        self.replay = self.enterContext(patch.object(export_checkpoint, "replay", side_effect=self.replay_history))
        self.enterContext(patch("builtins.print"))

    @staticmethod
    def write(path, data):
        path.write_text(json.dumps(data), encoding="utf-8")

    @staticmethod
    def request(chunk, differences):
        return {
            "chunk": chunk, "status": "FEATURES", "complete": True,
            "states": {"mismatches": differences, "total": 98304},
            "biomes": {"mismatches": 0, "total": 1536}, "source_order_differences": 0,
            "light_scored": False, "light_matches": None, "wg_presence_differences": 0,
            "metadata": {name: {"scored": True, "mismatches": 0}
                         for name in ("structures", "block_entity_payloads", "postprocessing")},
        }

    @staticmethod
    def history(name, requests):
        return {"name": name, "binary_sha256": "a" * 64, "source_zip_sha256": "d" * 64,
                "native_provenance_sha256": hashlib.sha256(name.encode()).hexdigest(),
                "comparison": f"wave/{name}/comparison.json", "comparison_sha256": "e" * 64,
                "comparator_sha256": "f" * 64, "requests": requests}

    def replay_history(self, directory):
        return copy.deepcopy(self.before if directory.name == "before" else self.histories[directory.name])

    def publish(self, *options):
        argv = ["export_checkpoint.py", "--tests", str(self.tests), "--wave", str(self.wave),
                "--output", str(self.output), *map(str, options)]
        with patch("sys.argv", argv):
            export_checkpoint.main()

    def assert_carry_rejected(self, message):
        self.write(self.carried, self.carried_record)
        with self.assertRaisesRegex(ValueError, message):
            self.publish("--carry-before-after", self.carried)
        self.assertFalse(self.output.exists())

    def test_unfinished_workspace_and_failed_log_cannot_publish(self):
        self.write(self.tests, {"state": "running", "exit_code": 0})
        with self.assertRaisesRegex(ValueError, "completed successful workspace run"):
            self.publish()
        self.write(self.tests, {"state": "finished", "exit_code": 0,
                               "command": ["cargo", "test", "--workspace"]})
        self.tests.with_suffix(".log").write_text(
            "test result: FAILED. 7 passed; 1 failed; 0 ignored\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "missing/failed target results"):
            self.publish()
        self.replay.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_unfinished_or_failed_wave_cannot_publish(self):
        for state, exit_code in (("running", 0), ("finished", 1)):
            with self.subTest(state=state, exit_code=exit_code):
                self.write(self.wave / "status.json", {"state": state, "cases": [
                    {"capture": "river", "exit_code": exit_code}
                ]})
                with self.assertRaisesRegex(ValueError, "unfinished or failed replays"):
                    self.publish()
                self.replay.assert_not_called()
                self.assertFalse(self.output.exists())

    def test_duplicate_history_cannot_inflate_counts(self):
        with self.assertRaisesRegex(ValueError, "duplicate replay history"):
            self.publish("--replay", self.wave / "river")
        self.assertFalse(self.output.exists())

    def test_mixed_frozen_executables_cannot_publish(self):
        self.histories["ridge"]["binary_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "one frozen executable"):
            self.publish()
        self.assertFalse(self.output.exists())

    def test_before_matching_specs_but_different_native_capture_is_rejected(self):
        self.before["native_provenance_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "identical native capture"):
            self.publish("--before", self.root / "before")
        self.assertFalse(self.output.exists())

    def test_before_reordered_requests_are_rejected(self):
        self.before["requests"].reverse()
        with self.assertRaisesRegex(ValueError, "same native request history"):
            self.publish("--before", self.root / "before")
        self.assertFalse(self.output.exists())

    def test_carried_matching_counts_but_spoofed_native_capture_is_rejected(self):
        self.carried_record["histories"][0]["native_provenance_sha256"] = "0" * 64
        self.assert_carry_rejected("different native inputs: river")

    def test_carried_changed_request_order_chunk_or_stage_is_rejected(self):
        original = copy.deepcopy(self.carried_record)
        for change in ("order", "chunk", "stage"):
            with self.subTest(change=change):
                self.carried_record = copy.deepcopy(original)
                requests = self.carried_record["histories"][0]["requests"]
                if change == "order":
                    requests.reverse()
                elif change == "chunk":
                    requests[0]["chunk"] = [99, 99]
                else:
                    requests[0]["status"] = "FULL"
                self.assert_carry_rejected("different native inputs: river")

    def test_carried_forged_request_count_is_rejected(self):
        self.carried_record["before_after"][0]["requests"] = 200
        self.assert_carry_rejected("different native inputs: river")

    def test_carried_missing_historical_or_current_history_is_rejected(self):
        original = copy.deepcopy(self.carried_record)
        for missing in ("historical", "current"):
            with self.subTest(missing=missing):
                self.carried_record = copy.deepcopy(original)
                if missing == "historical":
                    self.carried_record["histories"].pop(0)
                else:
                    self.carried_record["histories"][0]["name"] = "absent"
                    self.carried_record["before_after"][0]["history"] = "absent"
                self.assert_carry_rejected("carried history is absent from this wave")

    def test_carried_different_scored_result_is_rejected(self):
        self.carried_record["before_after"][0]["after_state_differences"] = 0
        self.assert_carry_rejected("historical result differs from current wave: river")

    def test_carried_forged_comparison_hash_is_rejected(self):
        self.carried_record["before_after"][0]["after_comparison_sha256"] = "0" * 64
        self.assert_carry_rejected("historical comparison identity differs: river")

    def test_carry_and_fresh_before_cannot_be_combined(self):
        with self.assertRaisesRegex(ValueError, "exclusive"):
            self.publish("--before", self.root / "before", "--carry-before-after", self.carried)
        self.assertFalse(self.output.exists())

    def test_fresh_before_after_uses_current_comparison_identity(self):
        self.publish("--before", self.root / "before")
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(result["before_after"], [{
            "history": "river", "requests": 2, "before_state_differences": 17,
            "after_state_differences": 3, "before_comparison_sha256": "b" * 64,
            "after_comparison_sha256": self.histories["river"]["comparison_sha256"],
        }])

    def test_carry_preserves_historical_hashes_and_labels_the_source(self):
        source_bytes = self.carried.read_bytes()
        self.publish("--carry-before-after", self.carried)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(result["before_after"], [{
            **self.carried_record["before_after"][0],
            "carried_from": "docs/metrics/published-history.json",
            "carried_from_sha256": hashlib.sha256(source_bytes).hexdigest(),
            "comparison_scope": "historical before/after, not recomputed for this wave",
        }])
        self.assertEqual(self.carried.read_bytes(), source_bytes)
        self.assertEqual(result["histories"], list(self.histories.values()))
        self.assertEqual(result["totals"]["histories"], 2)
        self.assertEqual(result["totals"]["requests"], 3)
        self.assertEqual(result["totals"]["state_differences"], 3)
        self.assertEqual(result["totals"]["state_observations"], 294912)
        self.assertEqual(result["tests"]["passed"], 9)
        self.assertEqual(result["tests"]["target_summaries"], 2)
        self.assertEqual(result["tests"]["ignored_tests"], ["skipped_case"])

    def test_carried_source_digest_uses_git_lf_content(self):
        published = json.dumps(self.carried_record, indent=2).encode("utf-8") + b"\n"
        local = published.replace(b"\n", b"\r\n")
        self.carried.write_bytes(local)
        self.publish("--carry-before-after", self.carried)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(result["before_after"][0]["carried_from_sha256"], hashlib.sha256(published).hexdigest())
        self.assertNotEqual(hashlib.sha256(local).hexdigest(), hashlib.sha256(published).hexdigest())
        self.assertEqual(self.carried.read_bytes(), local)
        self.assertNotIn(b"\r\n", self.output.read_bytes())


if __name__ == "__main__":
    unittest.main(verbosity=2)
