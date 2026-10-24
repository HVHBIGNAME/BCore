"""Offline integrity/admission checks for native feature capture and publication."""
from contextlib import redirect_stderr
from copy import deepcopy
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import native
import speleothem
import template_assets
import template_features


class NativeFeatureBoundaryTests(unittest.TestCase):
    def setUp(self):
        environment = patch.object(native, "environment", side_effect=AssertionError("native bootstrap invoked"))
        environment.start()
        self.addCleanup(environment.stop)

    def test_existing_or_outside_output_fails_before_native_bootstrap(self):
        for module in (speleothem, template_features):
            for path in (native.ROOT / "scripts/migration-26.2", native.ROOT / "outside-feature-output"):
                with self.subTest(module=module.__name__, path=path), \
                        patch.object(sys, "argv", [module.__name__, "capture", "--output", str(path)]), \
                        redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as failure:
                    module.main()
                self.assertEqual(failure.exception.code, 2)

    def test_repeat_requires_both_source_and_manifest_pin(self):
        for module in (speleothem, template_features):
            with self.subTest(module=module.__name__), patch.object(sys, "argv", [
                    module.__name__, "capture", "--output", str(native.ROOT / "target/feature-boundary-test"),
                    "--source", "target/original-capture"]), redirect_stderr(io.StringIO()), \
                    self.assertRaises(SystemExit) as failure:
                module.main()
            self.assertEqual(failure.exception.code, 2)

    def test_manifest_pin_must_match_before_reading_inputs(self):
        for module in (speleothem, template_features, template_assets):
            with self.subTest(module=module.__name__), patch.object(native, "sha", return_value="changed"), \
                    self.assertRaisesRegex(ValueError, "manifest changed"):
                module.verified(native.ROOT / "target/fake-native-feature", "expected")

    def test_pinned_manifest_cannot_escape_its_capture_directory(self):
        source = (native.ROOT / "target/fake-native-feature").resolve()
        manifest = {"jar_sha256": native.JAR_SHA256, "files_sha256": {"../outside.json": "digest"}}
        for module in (speleothem, template_features):
            with self.subTest(module=module.__name__), patch.object(native, "sha", return_value="digest"), \
                    patch.object(Path, "read_bytes", return_value=json.dumps(manifest).encode()), \
                    self.assertRaisesRegex(ValueError, "evidence changed"):
                module.verified(source, "digest")

    def test_promotion_requires_an_independent_repeat(self):
        for module in (speleothem, template_features):
            with self.subTest(module=module.__name__), patch.object(native, "environment", return_value=None), \
                    patch.object(module, "verified", return_value=({"independent_repeat": False}, {})), \
                    self.assertRaisesRegex(ValueError, "independent"):
                module.promote(native.ROOT / "target/fake-native-feature", "digest")

    def test_promotion_refuses_different_existing_bytes(self):
        for module in (speleothem, template_features):
            manifest = {"independent_repeat": True, "files_sha256": {module.FIXTURE: "expected"}}
            with self.subTest(module=module.__name__), patch.object(native, "environment", return_value=None), \
                    patch.object(module, "verified", return_value=(manifest, {})), \
                    patch.object(Path, "exists", return_value=True), patch.object(native, "sha", return_value="different"), \
                    self.assertRaisesRegex(ValueError, "different|Different"):
                module.promote(native.ROOT / "target/fake-native-feature", "digest")

    def test_template_capture_requires_full_coverage_and_native_boundary(self):
        result = {"minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
                  "samples": [{"explicit_section_marks": []}] * 153, "admission": [{}] * 12, "potent_sulfur_loads": [{}] * 55,
                  "gameplay_ticks": 0, "manager_source": "initialized-server-structure-manager"}
        template_features.identity(result)
        for key, changed in (("minecraft", "26.1"), ("protocol", 775), ("jar_sha256", "other"),
                             ("samples", [{}] * 152), ("admission", [{}] * 11),
                             ("potent_sulfur_loads", [{}] * 54), ("gameplay_ticks", 1),
                             ("manager_source", "direct-filesystem")):
            invalid = {**result, key: changed}
            with self.subTest(key=key), self.assertRaises(ValueError):
                template_features.identity(invalid)

    def test_template_capture_requires_typed_explicit_marks(self):
        result = {"minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
                  "samples": [{"explicit_section_marks": []}] * 153, "admission": [{}] * 12,
                  "potent_sulfur_loads": [{}] * 55, "gameplay_ticks": 0,
                  "manager_source": "initialized-server-structure-manager"}
        for marks in (None, False, [[1, 2]], [[1, False, 3]], [[1, 2, 3.0]]):
            invalid = {**result, "samples": [{"explicit_section_marks": marks}] * 153}
            with self.subTest(marks=marks), self.assertRaises(ValueError):
                template_features.identity(invalid)

    def test_effects_projection_requires_the_original_runtime_and_codec_results(self):
        sample = {"name": "native-spring", "seed": 17, "terrain": "stone", "origin": [15, 64, -17],
                  "write_calls": [[15, 64, -17, 1, 3, 1]], "next_i64": 42}
        original = {"minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
                    "gameplay_ticks": 0, "manager_source": "initialized-server-structure-manager",
                    "source_sha256": {"probe": "first"}, "samples": [sample], "admission": [True]}
        observed = {**original, "source_sha256": {"probe": "extended"},
                    "samples": [{**sample, "explicit_section_marks": [[15, 65, -17], [15, 65, -17]]}]}
        projected = template_features.project_effects(observed, original)
        self.assertEqual(projected["reference_sha256"], template_features.REFERENCE_SHA256)
        self.assertEqual(projected["samples"][0]["explicit_section_marks"], [[15, 65, -17], [15, 65, -17]])
        for invalid in ({**observed, "admission": [False]}, {**observed, "samples": []},
                        {**observed, "samples": [{**observed["samples"][0], "next_i64": 43}]}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                template_features.project_effects(invalid, original)


class FeatureTemplateProjectionTests(unittest.TestCase):
    def setUp(self):
        identity = {"minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
                    "states": [[0]], "defaults": {"minecraft:air": 0}, "water_fluid_id": 5}
        self.basis = deepcopy(identity)
        self.catalog = {**identity, "block_entities": {}, "templates": {}, "template_sources": {}}
        for name in template_assets.TEMPLATES:
            self.catalog["templates"][name] = {"size": [1, 1, 1], "palettes": [[{"Name": "minecraft:air"}]],
                                                "blocks": [[0, 0, 0, 0]], "entities": [], "data_version": 4903}
            self.catalog["template_sources"][name] = {
                "missing_from_jar": False, "path": f"data/minecraft/structure/{name.removeprefix('minecraft:')}.nbt"}

    def test_projection_preserves_ordered_native_arrays_and_source_identity(self):
        projected = template_assets.project(self.catalog, self.basis, ["first", "repeat"])
        self.assertEqual(projected["templates"], self.catalog["templates"])
        self.assertEqual(projected["template_sources"], self.catalog["template_sources"])
        self.assertEqual(projected["capture_manifest_sha256"], ["first", "repeat"])
        self.assertNotIn("states", projected)

    def test_registry_join_requires_identical_states_defaults_and_fluid_identity(self):
        for key, changed in (("states", [[1]]), ("defaults", {"minecraft:air": 1}), ("water_fluid_id", 4)):
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "registries differ"):
                template_assets.project({**self.catalog, key: changed}, self.basis, ["a", "b"])

    def test_missing_native_source_and_incomplete_inventory_fail_closed(self):
        name = next(iter(template_assets.TEMPLATES))
        changed = deepcopy(self.catalog)
        changed["template_sources"][name]["missing_from_jar"] = True
        with self.assertRaisesRegex(ValueError, "original source"):
            template_assets.project(changed, self.basis, ["a", "b"])
        del self.catalog["templates"][name]
        with self.assertRaisesRegex(ValueError, "inventory differs"):
            template_assets.project(self.catalog, self.basis, ["a", "b"])

    def test_dimensions_palette_indices_and_numeric_block_rows_are_admitted(self):
        name = next(iter(template_assets.TEMPLATES))
        for key, invalid in (("size", [0, 1, 1]), ("data_version", 4863), ("palettes", []),
                             ("blocks", [[-1, 0, 0, 0]]), ("blocks", [[0, 0, 0, 1]]),
                             ("blocks", [[False, 0, 0, 0]])):
            changed = deepcopy(self.catalog)
            changed["templates"][name][key] = invalid
            with self.subTest(key=key, value=invalid), self.assertRaises(ValueError):
                template_assets.project(changed, self.basis, ["a", "b"])


if __name__ == "__main__":
    unittest.main()
