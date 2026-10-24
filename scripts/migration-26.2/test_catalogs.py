"""Offline capture-boundary checks; no JVM, queue, network or generated files."""
import base64
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import runpy
import struct
import sys
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, Mock, patch

import catalogs


class CatalogBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.environment = patch.object(catalogs.native, "environment", side_effect=AssertionError("native environment invoked"))
        self.process = patch.object(catalogs.subprocess, "Popen", side_effect=AssertionError("subprocess invoked"))
        self.environment.start()
        self.process.start()
        self.addCleanup(self.environment.stop)
        self.addCleanup(self.process.stop)

    def stdout(self, value):
        path = Mock()
        path.stat.return_value.st_size = len(value.encode("utf-8"))
        path.read_text.return_value = value
        path.name = "capture.stdout.log"
        path.with_name.return_value.read_text.return_value = ""
        return path

    def test_mode_inventory_resolves_sources_without_duplicate_classes(self):
        self.assertEqual(len(catalogs.PROBES), 25)
        for mode, probe in catalogs.PROBES.items():
            with self.subTest(mode=mode):
                self.assertEqual(len({Path(name).name for name in probe.sources}), len(probe.sources))
                for name in probe.sources:
                    self.assertTrue((catalogs.native.ROOT / name).is_file(), name)

    def test_list_requires_no_native_environment(self):
        output = io.StringIO()
        with patch.object(sys, "argv", ["catalogs.py", "list"]), redirect_stdout(output):
            catalogs.main()
        self.assertIn('"spawn_handoff"', output.getvalue())

    def test_existing_and_outside_outputs_are_rejected_before_bootstrap(self):
        for path in (catalogs.native.ROOT / "scripts/migration-26.2", catalogs.native.ROOT / "outside-catalog-output"):
            with self.subTest(path=path), patch.object(sys, "argv", ["catalogs.py", "heightmap", "--output", str(path)]):
                with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                    catalogs.main()
                self.assertEqual(error.exception.code, 2)

    def test_spring_family_is_explicit_and_reaches_jigsaw_authentication(self):
        self.assertEqual(len(catalogs.FAMILIES), 5)
        self.assertNotIn("spring", catalogs.FAMILIES)
        arguments = ["catalogs.py", "jigsaw_assets", "--family", "spring", "--output",
                     str(catalogs.native.ROOT / "target/offline-spring-capture")]
        with patch.object(sys, "argv", arguments), patch.object(Path, "exists", return_value=False), \
                patch.object(Path, "is_dir", return_value=True), \
                self.assertRaisesRegex(AssertionError, "native environment invoked"):
            catalogs.main()

    def test_spring_family_on_non_jigsaw_and_unknown_family_fail_before_authentication(self):
        for mode, family in (("heightmap", "spring"), ("jigsaw_assets", "unknown_family")):
            arguments = ["catalogs.py", mode, "--family", family, "--output",
                         str(catalogs.native.ROOT / "target/offline-spring-capture")]
            with self.subTest(mode=mode, family=family), patch.object(sys, "argv", arguments), \
                    patch.object(Path, "exists", return_value=False), \
                    patch.object(Path, "is_dir", return_value=True), \
                    redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                catalogs.main()
            self.assertEqual(error.exception.code, 2)

    def test_probe_marker_and_conflicting_target_identity(self):
        payload = '[bootstrap] SCULK_REFERENCE={"minecraft":"26.2","protocol":776,"data":{"state_count":32366}}\n'
        self.assertEqual(catalogs.read_result(self.stdout(payload), "SCULK_REFERENCE=", 1024)["data"]["state_count"], 32366)
        with self.assertRaisesRegex(ValueError, "refusing to relabel"):
            catalogs.read_result(self.stdout(payload.replace('"26.2"', '"26.1"')), "SCULK_REFERENCE=", 1024)
        with self.assertRaisesRegex(ValueError, "exactly one"):
            catalogs.read_result(self.stdout(payload + payload), "SCULK_REFERENCE=", 2048)

    def test_complete_ranges_and_missing_biomes_fail_closed(self):
        catalogs.validate_ranges([[0, 10], [10, 32366]], "fixture")
        for rows in ([[0, 32365]], [[0, 10], [11, 32366]], [[0, 11], [10, 32366]], [[0, 32367]]):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                catalogs.validate_ranges(rows, "fixture")
        with self.assertRaises(ValueError):
            catalogs.validate_catalog("surface_climates", {str(i): ["00000000", False] for i in range(65)})

    def test_typed_nbt_retains_numeric_widths_and_empty_list_type(self):
        def name(text):
            raw = text.encode("utf-8")
            return struct.pack(">H", len(raw)) + raw

        raw = (b"\x0a\x00\x00" + b"\x04" + name("seed") + struct.pack(">q", 9223372036854775807)
               + b"\x02" + name("count") + struct.pack(">h", -12)
               + b"\x09" + name("Items") + b"\x0a" + struct.pack(">i", 0)
               + b"\x07" + name("bytes") + struct.pack(">i", 3) + b"\x80\x00\x7f\x00")
        helper = runpy.run_path(str(catalogs.native.ROOT / "scripts/jigsaw-reference/capture.py"))
        codec = SimpleNamespace(NbtReader=helper["NbtReader"])
        data = catalogs.decode_nbt(codec, {"full": {"nbt": base64.b64encode(raw).decode("ascii")}})
        compound = data["full"]["nbt"]["Compound"]
        self.assertEqual(compound["seed"], {"Long": 9223372036854775807})
        self.assertEqual(compound["count"], {"Short": -12})
        self.assertEqual(compound["Items"], {"List": {"element_type": 10, "values": []}})
        self.assertEqual(compound["bytes"], {"ByteArray": [-128, 0, 127]})

    def test_changed_historical_source_pin_is_rejected(self):
        name = "scripts/TreeReference.java"
        prepared = {"baseline_snapshot": {"source_sha256": {name: "1" * 64}}}
        with patch.object(catalogs.native, "PREPARATION") as preparation:
            preparation.read_bytes.return_value = json.dumps(prepared).encode("utf-8")
            with self.assertRaisesRegex(ValueError, "historical probe/helper changed"):
                catalogs.preserved_source_hashes({name: "0" * 64})

    def process_fixture(self, stdout):
        output = MagicMock()
        out, err = io.BytesIO(), io.BytesIO()
        paths = {}
        for name, stream in (("capture.stdout.log", out), ("capture.stderr.log", err)):
            path = MagicMock()
            path.open.return_value.__enter__.return_value = stream
            paths[name] = path
        output.__truediv__.side_effect = lambda name: paths.get(name, MagicMock())
        process = MagicMock()
        process.__enter__.return_value = process
        process.stdout = io.BytesIO(stdout)
        process.stderr = io.BytesIO()
        process.returncode = -9
        stopped = threading.Event()
        process.poll.side_effect = lambda: -9 if stopped.is_set() else None
        process.kill.side_effect = stopped.set
        return output, out, err, process

    def test_log_overflow_kills_owned_process_and_retains_exact_bound(self):
        output, out, err, process = self.process_fixture(b"x" * 8192)
        with patch.object(catalogs.subprocess, "Popen", return_value=process), \
                patch.object(catalogs.native, "write_json") as record, patch.object(catalogs.native, "sha", return_value="digest"):
            with self.assertRaisesRegex(RuntimeError, "bounded logs"):
                catalogs.bounded_execute(["mock"], output, "capture", timeout=10, log_limit=1024, environment={})
        process.kill.assert_called_once()
        self.assertEqual(len(out.getvalue()), 1024)
        self.assertEqual(err.getvalue(), b"")
        result = record.call_args_list[-1].args[1]
        self.assertTrue(result["truncated"])
        self.assertEqual(result["observed_stream_bytes"]["stdout"], 8192)

    def test_deadline_kills_owned_process_and_records_failure(self):
        output, out, err, process = self.process_fixture(b"")
        with patch.object(catalogs.subprocess, "Popen", return_value=process), \
                patch.object(catalogs.native, "write_json") as record, patch.object(catalogs.native, "sha", return_value="digest"), \
                patch.object(catalogs.time, "monotonic", side_effect=(0, 2, 3)):
            with self.assertRaisesRegex(RuntimeError, "exceeded 1 seconds"):
                catalogs.bounded_execute(["mock"], output, "capture", timeout=1, log_limit=1024, environment={})
        process.kill.assert_called_once()
        result = record.call_args_list[-1].args[1]
        self.assertFalse(result["truncated"])
        self.assertEqual(result["failure"], "subprocess exceeded 1 seconds")


if __name__ == "__main__":
    unittest.main()
