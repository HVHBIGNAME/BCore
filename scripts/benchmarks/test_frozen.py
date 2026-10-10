"""Exercise frozen comparisons end to end with inert binaries and mocked launches."""
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import call, patch

import compare_frozen
import provenance
import run


class FrozenComparisonTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(dir=run.ROOT / "target")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.environment = {"policy": "test-only", "fixed": {"CARGO_BUILD_JOBS": "1"}}
        self.env = {"CARGO_BUILD_JOBS": "1"}
        self.fingerprints = [
            {"chunk": pos, "marks": 0, **{key: "0" * 64 for key in run.HASH_FIELDS}}
            for pos in run.CHUNKS
        ]
        self.before = self.make_run("before")
        self.after = self.make_run("after")
        self.output_count = 0
        self.execute = self.enterContext(patch.object(compare_frozen, "execute", side_effect=self.launch))
        self.sleep = self.enterContext(patch.object(compare_frozen.time, "sleep"))
        self.enterContext(patch.object(compare_frozen, "controlled_environment",
                                      return_value=(self.env, self.environment)))
        self.enterContext(patch("builtins.print"))

    def make_run(self, label):
        directory = self.root / label
        directory.mkdir()
        executable = directory / "inert-binary"
        executable.write_bytes(f"not executable: {label}".encode())
        archive = directory / "source.zip"
        archive.write_bytes(f"opaque frozen source: {label}".encode())
        manifest = directory / "source-manifest.json"
        manifest.write_text(json.dumps({"snapshot": label}), encoding="utf-8")
        identity = {key: "test-only" for key in (
            "harness_source_sha256", "native_jar_sha256", "native_libraries_sha256",
            "native_classes_sha256", "runtime_sha256", "rustc", "cargo", "java", "javac",
            "java_flags", "cargo_config_sha256",
        )}
        identity.update(environment=self.environment, executable=executable.name,
                        bcore_executable_sha256=provenance.sha(executable),
                        source_archive_sha256=provenance.sha(archive),
                        source_manifest_sha256=provenance.sha(manifest))
        record = {
            "schema": run.SCHEMA, "scope": run.SCOPE, "contract": run.CONTRACT,
            "seed": "846692123413862008", "chunks": run.CHUNKS, "workers": [1, 4],
            "hardware": {"logical_processors": 8}, "processes_per_configuration": 3,
            "warmup_batches_per_process": 1, "measured_batches_per_process": 3,
            "provenance": identity, "fingerprints": self.fingerprints,
            "verified_matching_outputs": True,
        }
        path = directory / "results.json"
        provenance.write_json(path, record)
        return path

    def output(self):
        self.output_count += 1
        path = self.root / f"comparison-{self.output_count}"
        path.mkdir()
        return path

    def launch(self, command, cwd, log, timeout, env):
        self.assertEqual(timeout, 900)
        self.assertEqual(env, self.env)
        self.assertEqual(Path(command[0]).parent, cwd)
        request = json.loads(Path(command[1]).read_text(encoding="utf-8"))
        self.assertEqual(request["chunks"], run.CHUNKS)
        label = Path(command[0]).stem
        self.assertEqual(Path(command[0]).read_bytes(), f"not executable: {label}".encode())
        process = int(log.stem.rsplit("-", 1)[1])
        times = {
            "before": [[1, 9, 100], [8, 10, 11], [2, 12, 14]],
            "after": [[1, 2, 90], [3, 4, 80], [5, 6, 70]],
        }
        sample = {
            "schema": run.SCHEMA, "scope": run.SCOPE, "engine": "BCore",
            "seed": request["seed"], "workers": request["workers"],
            "warmup_batches": request["warmup_batches"], "chunks_per_batch": len(run.CHUNKS),
            "seconds": [value * request["workers"] for value in times[label][process - 1]],
            "warmup_seconds": [100000], "fingerprints": self.fingerprints,
        }
        log.write_text("startup message\nNOISE_BENCHMARK=" + json.dumps(sample) + "\n",
                       encoding="utf-8")

    def assert_rejected_before_launch(self, message):
        output = self.output()
        with self.assertRaisesRegex(ValueError, message):
            compare_frozen.compare(self.before, self.after, output)
        self.execute.assert_not_called()
        self.sleep.assert_not_called()
        self.assertFalse((output / "results.json").exists())

    def test_unfinished_either_input_cannot_launch(self):
        for path in (self.before, self.after):
            original = path.read_text(encoding="utf-8")
            for flag in (False, None, 1):
                with self.subTest(input=path.parent.name, completion=flag):
                    record = json.loads(original)
                    record["verified_matching_outputs"] = flag
                    path.write_text(json.dumps(record), encoding="utf-8")
                    self.assert_rejected_before_launch("not a completed matched-output run")
            path.write_text(original, encoding="utf-8")

    def test_incompatible_contract_or_runtime_cannot_launch(self):
        original = self.after.read_text(encoding="utf-8")
        for section, field in (("contract", "timed"), ("provenance", "runtime_sha256")):
            with self.subTest(section=section):
                record = json.loads(original)
                record[section][field] = "different measurement"
                self.after.write_text(json.dumps(record), encoding="utf-8")
                self.assert_rejected_before_launch(f"comparison .* differs: {section if section == 'contract' else field}")

    def test_different_input_fingerprints_cannot_launch(self):
        record = json.loads(self.after.read_text(encoding="utf-8"))
        record["fingerprints"][0]["blocks_sha256"] = "1" * 64
        self.after.write_text(json.dumps(record), encoding="utf-8")
        self.assert_rejected_before_launch("input fingerprints differ")

    def test_current_environment_drift_cannot_launch(self):
        with patch.object(compare_frozen, "controlled_environment", return_value=(self.env, {})):
            self.assert_rejected_before_launch("current environment policy differs")

    def test_changed_input_artifacts_cannot_launch(self):
        for name in ("inert-binary", "source.zip", "source-manifest.json"):
            with self.subTest(artifact=name):
                artifact = self.after.parent / name
                original = artifact.read_bytes()
                artifact.write_bytes(b"tampered")
                try:
                    self.assert_rejected_before_launch("artifact changed or missing")
                finally:
                    artifact.write_bytes(original)

    def test_frozen_copy_tampered_before_launch_is_rejected(self):
        output = self.output()

        def write_and_tamper(path, value, **kwargs):
            provenance.write_json(path, value, **kwargs)
            if path.name.endswith(".request.json"):
                binary = output / ("before.exe" if os.name == "nt" else "before")
                binary.chmod(stat.S_IREAD | stat.S_IWRITE)
                binary.write_bytes(b"tampered after freezing")

        with patch.object(compare_frozen, "write_json", side_effect=write_and_tamper):
            with self.assertRaisesRegex(ValueError, "artifact changed or missing"):
                compare_frozen.compare(self.before, self.after, output)
        self.execute.assert_not_called()
        self.sleep.assert_not_called()
        self.assertFalse((output / "results.json").exists())

    def test_artifacts_tampered_during_launch_are_rejected_before_progress(self):
        for name in ("frozen-binary", "source.zip", "source-manifest.json", "results.json"):
            with self.subTest(artifact=name):
                output = self.output()
                original = None
                artifact = None

                def launch_and_tamper(command, cwd, log, timeout, env):
                    nonlocal artifact, original
                    self.launch(command, cwd, log, timeout, env)
                    artifact = Path(command[0]) if name == "frozen-binary" else self.before.parent / name
                    original = artifact.read_bytes()
                    artifact.chmod(stat.S_IREAD | stat.S_IWRITE)
                    artifact.write_bytes(b"tampered while running")

                self.execute.reset_mock()
                self.execute.side_effect = launch_and_tamper
                try:
                    with self.assertRaisesRegex(ValueError, "artifact changed or missing"):
                        compare_frozen.compare(self.before, self.after, output)
                    self.execute.assert_called_once()
                    self.sleep.assert_not_called()
                    self.assertFalse((output / "progress.json").exists())
                    self.assertFalse((output / "results.json").exists())
                finally:
                    if artifact is not None:
                        artifact.write_bytes(original)

    def test_validly_shaped_but_different_launched_output_is_rejected(self):
        def mismatching_launch(command, cwd, log, timeout, env):
            self.launch(command, cwd, log, timeout, env)
            sample = json.loads(log.read_text(encoding="utf-8").split("NOISE_BENCHMARK=", 1)[1])
            sample["fingerprints"][0]["ocean_floor_wg_sha256"] = "f" * 64
            log.write_text("NOISE_BENCHMARK=" + json.dumps(sample), encoding="utf-8")

        self.execute.side_effect = mismatching_launch
        output = self.output()
        with self.assertRaisesRegex(ValueError, "block/mark/WG mismatch"):
            compare_frozen.compare(self.before, self.after, output)
        self.execute.assert_called_once()
        self.sleep.assert_not_called()
        self.assertFalse((output / "progress.json").exists())
        self.assertFalse((output / "results.json").exists())

    def test_success_alternates_frozen_launches_and_publishes_process_medians(self):
        output = self.output()
        compare_frozen.compare(self.before, self.after, output)
        result = json.loads((output / "results.json").read_text(encoding="utf-8"))
        expected_order = [(workers, process, variant) for workers in (1, 4)
                          for process, variants in ((1, ("before", "after")),
                                                    (2, ("after", "before")),
                                                    (3, ("before", "after")))
                          for variant in variants]
        self.assertEqual([(s["workers"], s["process"], s["variant"]) for s in result["samples"]],
                         expected_order)
        self.assertEqual([args.args[2].stem for args in self.execute.call_args_list],
                         [f"{variant}-{workers}w-{process}" for workers, process, variant in expected_order])
        self.assertEqual(self.sleep.call_args_list, [call(1)] * 12)
        self.assertTrue(result["verified_matching_outputs"])
        self.assertEqual(result["fingerprints"], self.fingerprints)
        for row in result["summary"]:
            workers = row["workers"]
            self.assertEqual(row["before"]["process_median_seconds"], [9 * workers, 10 * workers, 12 * workers])
            self.assertEqual(row["after"]["process_median_seconds"], [2 * workers, 4 * workers, 6 * workers])
            self.assertAlmostEqual(row["before"]["chunks_per_second"], 8 / (10 * workers))
            self.assertAlmostEqual(row["after"]["chunks_per_second"], 8 / (4 * workers))
            self.assertAlmostEqual(row["speedup"], 2.5)
        for sample in result["samples"]:
            self.assertTrue(sample["artifacts_verified_before_after"])
            self.assertEqual(sample["request_sha256"], provenance.sha(Path(sample["command"][1])))
        for label, path in (("before", self.before), ("after", self.after)):
            self.assertEqual(result["inputs"][label]["results_sha256"], provenance.sha(path))


if __name__ == "__main__":
    unittest.main(verbosity=2)
