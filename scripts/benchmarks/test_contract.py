"""Focused failure-path checks; real native/Rust equivalence is tested by run.py."""
import copy
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import provenance
import run


class ContractTests(unittest.TestCase):
    def request(self):
        return {"seed": "846692123413862008", "workers": 4, "warmup_batches": 2, "measured_batches": 2}

    def sample(self):
        request = self.request()
        return {"schema": 2, "scope": run.SCOPE, "engine": "Vanilla 26.1", "seed": request["seed"],
                "workers": 4, "warmup_batches": 2, "chunks_per_batch": 8,
                "native_executor": "ForkJoinPool", "available_processors": 5,
                "seconds": [1.0, 3.0], "warmup_seconds": [100.0, 200.0],
                "fingerprints": [{"chunk": p, "marks": 0, **{key: "0" * 64 for key in run.HASH_FIELDS}}
                                 for p in run.CHUNKS]}

    def test_rejects_old_contract_and_missing_wg_maps(self):
        good = self.sample()
        run.validate_sample(good, "Vanilla 26.1", self.request())
        for field in ("schema", "scope", "warmup_seconds"):
            bad = copy.deepcopy(good)
            del bad[field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                run.validate_sample(bad, "Vanilla 26.1", self.request())
        for field in ("world_surface_wg_sha256", "ocean_floor_wg_sha256"):
            bad = copy.deepcopy(good)
            del bad["fingerprints"][3][field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                run.validate_sample(bad, "Vanilla 26.1", self.request())

    def test_rejects_wrong_executor_chunk_order_and_nonfinite_times(self):
        for edit in (lambda s: s.update(available_processors=16),
                     lambda s: s.update(workers=2),
                     lambda s: s["fingerprints"].reverse(),
                     lambda s: s.update(seconds=[float("nan"), 1]),
                     lambda s: s.update(warmup_seconds=[0, 1])):
            bad = self.sample()
            edit(bad)
            with self.assertRaises(ValueError):
                run.validate_sample(bad, "Vanilla 26.1", self.request())

    def test_summary_excludes_warmup_and_uses_process_medians(self):
        samples = []
        for engine in ("Vanilla 26.1", "BCore"):
            for values in ([1, 3], [10, 12], [5, 7]):
                samples.append({"engine": engine, "workers": 4, "seconds": values,
                                "warmup_seconds": [10000, 20000]})
        for row in run.summaries(samples, [4]):
            self.assertEqual(row["median_batch_seconds"], 6)
            self.assertEqual(row["min_process_median_seconds"], 2)
            self.assertEqual(row["max_process_median_seconds"], 11)

    def test_comparison_rejects_boundary_harness_and_runtime_changes(self):
        current = {key: "same" for key in ("schema", "scope", "seed", "chunks", "workers", "hardware",
                   "processes_per_configuration", "warmup_batches_per_process", "measured_batches_per_process")}
        current["contract"] = run.CONTRACT
        current["provenance"] = {key: "same" for key in ("harness_source_sha256", "native_jar_sha256",
            "native_libraries_sha256", "native_classes_sha256", "runtime_sha256", "rustc", "cargo", "java",
            "javac", "environment", "java_flags", "cargo_config_sha256")}
        baseline = copy.deepcopy(current)
        baseline["verified_matching_outputs"] = True
        run.compare_contract(baseline, current)
        for section, field in ((None, "scope"), (None, "warmup_batches_per_process"),
                               ("provenance", "harness_source_sha256"), ("provenance", "java_flags")):
            bad = copy.deepcopy(baseline)
            (bad[section] if section else bad)[field] = "changed"
            with self.subTest(field=field), self.assertRaises(ValueError):
                run.compare_contract(bad, current)

    def test_removed_overrides_do_not_leak_values(self):
        with patch.dict(os.environ, {"JAVA_TOOL_OPTIONS": "private-token", "RUSTFLAGS": "private-value",
                                     "BCORE_DATAPACK": "private-path", "CARGO_PROFILE_RELEASE_OPT_LEVEL": "0"}):
            env, record = provenance.controlled_environment()
        self.assertNotIn("JAVA_TOOL_OPTIONS", env)
        self.assertNotIn("RUSTFLAGS", env)
        self.assertNotIn("BCORE_DATAPACK", env)
        self.assertNotIn("CARGO_PROFILE_RELEASE_OPT_LEVEL", env)
        self.assertNotIn("private-", json.dumps(record))
        self.assertEqual(env["CARGO_BUILD_JOBS"], "1")

    def test_snapshot_contains_uncommitted_assets_and_detects_tampering(self):
        with tempfile.TemporaryDirectory(dir=run.ROOT / "target") as temp:
            root, output = Path(temp) / "repo", Path(temp) / "run"
            root.mkdir(); output.mkdir()
            inputs = {*provenance.HARNESS_SOURCES, "Cargo.toml", "Cargo.lock", "rust-toolchain.toml",
                      "crates/bcore-core/src/new_untracked.rs", "crates/bcore-worldgen/data/new_asset.bin",
                      "crates/bcore-worldgen/build.rs"}
            for name in inputs:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(name.encode())
            private = root / "crates/bcore-worldgen/.env"
            private.write_text("private-token", encoding="utf-8")
            manifest = provenance.source_hashes(root)
            self.assertEqual(set(manifest), inputs)
            source, record = provenance.snapshot(root, output, manifest)
            self.assertEqual(record["source_archive_sha256"], provenance.sha(output / "source.zip"))
            with zipfile.ZipFile(output / "source.zip") as archive:
                self.assertEqual(set(archive.namelist()), inputs)
                for name in inputs:
                    self.assertEqual(archive.read(name), (source / name).read_bytes())
            changed = source / "crates/bcore-worldgen/data/new_asset.bin"
            changed.chmod(stat.S_IWRITE | stat.S_IREAD)
            changed.write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                provenance.verify_files(source, manifest)

    def test_frozen_binary_is_independent_of_shared_build_output(self):
        with tempfile.TemporaryDirectory(dir=run.ROOT / "target") as temp:
            source, frozen = Path(temp) / "shared.exe", Path(temp) / "frozen.exe"
            source.write_bytes(b"original executable")
            digest = provenance.freeze_copy(source, frozen)
            source.write_bytes(b"a different build")
            provenance.verify_files(Path(temp), {"frozen.exe": digest})
            self.assertNotEqual(provenance.sha(source), digest)
            frozen.chmod(stat.S_IREAD | stat.S_IWRITE)
            frozen.write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                provenance.verify_files(Path(temp), {"frozen.exe": digest})


if __name__ == "__main__":
    unittest.main(verbosity=2)
