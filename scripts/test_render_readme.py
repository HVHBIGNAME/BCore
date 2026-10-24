"""Publication failure paths using synthetic target labels; no native execution."""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

import render_readme as render


class BenchmarkPublicationTests(unittest.TestCase):
    def measurement(self):
        data = render.load(render.BENCHMARK)
        data.update(minecraft="26.2", protocol=776)
        data["provenance"].update(native_version={"name": "26.2", "protocol_version": 776, "world_version": 4903},
            native_jar_sha256="183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8")
        for row in data["summary"] + data["samples"]:
            if row["engine"] == "Vanilla 26.1":
                row["engine"] = "Vanilla 26.2"
        for sample in data["samples"]:
            if sample["engine"] == "Vanilla 26.2":
                sample.update(minecraft="26.2", protocol=776, data_version=4903,
                              launched_artifact_sha256=data["provenance"]["native_jar_sha256"])
        return data

    def test_import_preserves_samples_and_records_original_capture_hash(self):
        data = self.measurement()
        with tempfile.TemporaryDirectory(dir=render.ROOT / "target") as temp:
            source, destination = Path(temp) / "capture.json", Path(temp) / "published.json"
            source.write_text(json.dumps(data), encoding="utf-8")
            render.import_benchmark(source, destination)
            published = render.load(destination)
            self.assertEqual(published["samples"], data["samples"])
            self.assertEqual(published["summary"], data["summary"])
            self.assertEqual(published["capture_results_sha256"], hashlib.sha256(source.read_bytes()).hexdigest())

    def test_incomplete_mixed_or_inconsistent_runs_cannot_publish(self):
        edits = (
            lambda d: d["samples"].pop(),
            lambda d: d["samples"].append(deepcopy(d["samples"][0])),
            lambda d: d["samples"][0].update(engine="Vanilla 26.1"),
            lambda d: d["samples"][0].update(protocol=775),
            lambda d: d["samples"][0].update(warmup_seconds=[]),
            lambda d: d["samples"][0].update(available_processors=16),
            lambda d: d["samples"][0]["seconds"].__setitem__(0, float("nan")),
            lambda d: d["summary"][0].update(chunks_per_second=1),
            lambda d: d.update(verified_matching_outputs=False),
            lambda d: d["provenance"].update(native_jar_sha256="historical jar"),
            lambda d: d["provenance"]["native_version"].update(protocol_version=775),
            lambda d: d["samples"][0].update(artifacts_verified_before_after=False),
            lambda d: d["samples"][0].update(launched_artifact_sha256="other executable"),
            lambda d: d["fingerprints"].pop(),
            lambda d: d["fingerprints"][0].update(blocks_sha256="missing"),
        )
        with tempfile.TemporaryDirectory(dir=render.ROOT / "target") as temp:
            for index, edit in enumerate(edits):
                with self.subTest(index=index):
                    data = self.measurement()
                    edit(data)
                    source, destination = Path(temp) / f"capture-{index}.json", Path(temp) / f"published-{index}.json"
                    source.write_text(json.dumps(data), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        render.import_benchmark(source, destination)
                    self.assertFalse(destination.exists())

    def test_publication_refuses_to_replace_existing_measurement(self):
        with tempfile.TemporaryDirectory(dir=render.ROOT / "target") as temp:
            source, destination = Path(temp) / "capture.json", Path(temp) / "published.json"
            source.write_text(json.dumps(self.measurement()), encoding="utf-8")
            destination.write_bytes(b"retained measurement")
            with self.assertRaises(FileExistsError):
                render.import_benchmark(source, destination)
            self.assertEqual(destination.read_bytes(), b"retained measurement")

    def test_chart_uses_the_actual_native_target_label(self):
        contents = render.performance(self.measurement())
        ET.fromstring(contents)
        self.assertIn("Vanilla 26.2", contents)
        self.assertNotIn("Vanilla 26.1", contents)
        self.assertIn("Vanilla 26.1", render.performance(render.load(render.BENCHMARK)))


if __name__ == "__main__":
    unittest.main(verbosity=2)
