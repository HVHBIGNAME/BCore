"""Regression checks for build/runtime attribution of saved Cargo test logs."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from test_timings import summarize


class TestTimingsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parents[1] / "target")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.log = self.root / "tests.log"

    def test_text_log_separates_build_queue_and_target_time(self):
        raw = (
            "    Finished `release` profile [optimized] target(s) in 2m 3.50s\r\n"
            "     Running unittests src\\lib.rs (library.exe)\r\n"
            "test result: ok. 6 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 2.25s\r\n"
            "   Doc-tests example\r\n"
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s\r\n"
        ).encode()
        self.log.write_bytes(raw)
        status = self.root / "status.json"
        status.write_text(json.dumps(dict(job="baseline", command=["cargo", "test"], exit_code=0,
                                         started=100, execution_started=110, finished=240)))
        result = summarize(self.log, status)
        self.assertEqual(result["log_sha256"], hashlib.sha256(raw).hexdigest())
        self.assertEqual(result["build_seconds"], [123.5])
        self.assertEqual(result["harness_seconds"], 2.5)
        self.assertEqual(result["queue_wait_seconds"], 10)
        self.assertEqual(result["execution_seconds"], 130)
        self.assertEqual(result["unattributed_seconds"], 4)
        self.assertEqual(result["totals"]["passed"], 7)
        self.assertEqual(result["totals"]["ignored"], 1)
        self.assertEqual(result["target_count"], 2)
        self.assertEqual(result["targets"][1]["target"], "Doc-tests example")

    def test_json_test_times_are_not_added_to_parallel_suite_wall_time(self):
        events = [
            {"reason": "compiler-artifact"},
            {"type": "suite", "event": "started", "test_count": 3},
            {"type": "test", "event": "ok", "name": "slow", "exec_time": 4},
            {"type": "test", "event": "failed", "name": "fast", "exec_time": 2},
            {"type": "test", "event": "ignored", "name": "manual"},
            {"type": "suite", "event": "failed", "passed": 1, "failed": 1, "ignored": 1,
             "measured": 0, "filtered_out": 5, "exec_time": 4.1},
        ]
        self.log.write_text("\n".join(map(json.dumps, events)))
        result = summarize(self.log)
        self.assertEqual(result["harness_seconds"], 4.1)
        self.assertEqual(result["totals"], dict(passed=1, failed=1, ignored=1, measured=0, filtered_out=5))
        self.assertEqual([t["name"] for t in result["tests"]], ["slow", "fast", "manual"])
        self.assertIsNone(result["tests"][2]["seconds"])

    def test_partial_log_and_running_status_do_not_invent_completed_tests(self):
        self.log.write_text('Finished `release` profile [optimized] target(s) in 750ms\n'
                            '{ "type": "suite", "event": "started", "test_count": 11 }\n'
                            '{ "type": "test", "event":')
        status = self.root / "status.json"
        status.write_text(json.dumps(dict(job="running", command=["cargo", "test"], started=100,
                                         execution_started=120)))
        result = summarize(self.log, status)
        self.assertEqual(result["build_seconds"], [0.75])
        self.assertEqual(result["target_count"], 0)
        self.assertEqual(result["tests"], [])
        self.assertNotIn("execution_seconds", result)
        self.assertIsNone(result["exit_code"])


if __name__ == "__main__":
    unittest.main()
