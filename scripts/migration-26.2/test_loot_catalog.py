"""Offline loot capture/admission tests; native launches are explicitly blocked."""
from contextlib import redirect_stderr, redirect_stdout
from copy import deepcopy
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import loot_catalog
import loot_catalog_data as data
import loot_catalog_fixture as fixture
import promote_loot_26_2 as promotion


def exports():
    identity = {"minecraft": "26.2", "protocol": 776, "jar_sha256": loot_catalog.native.JAR_SHA256}
    types = {name: [] for name in ("DATA_COMPONENT_TYPE", "LOOT_POOL_ENTRY_TYPE", "LOOT_FUNCTION_TYPE",
                                   "LOOT_CONDITION_TYPE", "LOOT_NUMBER_PROVIDER_TYPE", "LOOT_NBT_PROVIDER_TYPE",
                                   "LOOT_SCORE_PROVIDER_TYPE")}
    types["DATA_COMPONENT_TYPE"] = [
        {"id": 0, "key": "minecraft:max_stack_size", "persistent": True},
        {"id": 1, "key": "minecraft:transient_test", "persistent": False}]
    items = [{"id": index, "key": "minecraft:" + key, "max_stack_size": 64,
              "components": {"persistent": {"minecraft:max_stack_size": 64}, "unserialized_transient_types": []}}
             for index, key in enumerate(("stone", "book"))]
    potions = [{"id": 0, "key": "minecraft:healing", "effects": []}]
    tables = {"minecraft:chests/fixture": {"type": "minecraft:chest", "pools": []},
              "minecraft:blocks/fixture": {"pools": []}}
    counts = {"items": 2, "item_tags": 1, "potions": 1, "loaded_tables": 2, "chest_and_trial_tables": 1,
              "enchantments": 1, "enchantments_tags": 1, "instruments": 1, "instruments_tags": 1,
              "registry_types": {key: len(rows) for key, rows in types.items()}}
    values = {"items.json": {"items_native_order": items, "tags": [{
        "tag": "minecraft:books", "members_native_order": ["minecraft:book", "minecraft:stone"]}]},
        "registry-types.json": types, "potions.json": potions, "tables.json": tables,
        "loot-catalog.json": {**identity, "schema": data.CATALOG_SCHEMA, "mode": "catalog", "gameplay_ticks": 0,
                              "data_version": 4903, "counts": counts, "case_count": 0, "event_count": 0,
                              "lookup_source": "MinecraftServer.reloadableRegistries().lookup()",
                              "level_class": "net.minecraft.server.level.ServerLevel",
                              "server_class": "net.minecraft.server.dedicated.DedicatedServer",
                              "native_version": {"name": "26.2", "protocol_version": 776, "world_version": 4903}},
        "loaded-tables.json": {"all_loaded_keys": sorted(tables), "chest_and_trial_tables": [
            {"key": "minecraft:chests/fixture", "native_codec": tables["minecraft:chests/fixture"]}]},
        "table-inventory.json": {**identity, "all_jar_keys": sorted(tables),
                                 "chest_and_trial_keys": ["minecraft:chests/fixture"]}}
    for filename, key in (("enchantment.json", "minecraft:unbreaking"), ("instrument.json", "minecraft:ponder_goat_horn")):
        values[filename] = {"values_native_order": [{"key": key, "value": {}}], "tags": [
            {"tag": "minecraft:test", "members_native_order": [key]}]}
    registry = {**identity, "data_version": 4903, "registries": {
        name: [{"id": row["id"], "key": row["key"]} for row in rows]
        for name, rows in (("ITEM", items), ("POTION", potions), ("DATA_COMPONENT_TYPE", types["DATA_COMPONENT_TYPE"]))}}
    return values, registry


class LootCatalogTests(unittest.TestCase):
    def setUp(self):
        for target in (patch.object(loot_catalog.native, "environment", side_effect=AssertionError("native environment invoked")),
                       patch.object(loot_catalog.catalogs.subprocess, "Popen", side_effect=AssertionError("subprocess invoked"))):
            target.start()
            self.addCleanup(target.stop)

    def test_list_does_not_bootstrap_or_write_outputs(self):
        output = io.StringIO()
        with patch.object(sys, "argv", ["loot_catalog.py", "list"]), redirect_stdout(output):
            loot_catalog.main()
        self.assertIn('"default_input_cases": 137', output.getvalue())
        self.assertIn('"gameplay_ticks": 0', output.getvalue())

    def test_conflicting_output_and_incomplete_repeat_pin_fail_before_native(self):
        for arguments in (["catalog", "--output", str(loot_catalog.native.ROOT / "scripts")],
                          ["fill", "--repeat-of", "target/previous"],
                          ["catalog", "--output", "target/new-loot", "--max-log-mib", "0"]):
            with self.subTest(arguments=arguments), patch.object(sys, "argv", ["loot_catalog.py", *arguments]):
                with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                    loot_catalog.main()
                self.assertEqual(error.exception.code, 2)

    def test_historical_java_and_verifier_sources_keep_their_original_hashes(self):
        for name, expected in {**data.HISTORICAL_JAVA, **fixture.VERIFIER_SOURCES}.items():
            with self.subTest(name=name):
                self.assertEqual(loot_catalog.native.sha(loot_catalog.native.ROOT / name), expected)
        self.assertEqual(len({Path(name).name for name in loot_catalog.SOURCES}), len(loot_catalog.SOURCES))

    def test_historical_matrix_reuses_only_137_explicit_inputs(self):
        spec, authority = fixture.input_spec("fill", None)
        self.assertEqual(len(spec["cases"]), 137)
        self.assertEqual(authority["source_sha256"], fixture.HISTORICAL_SHA256)
        self.assertEqual(authority["authority"], "historical-26.1-input-replay-on-26.2")
        self.assertNotIn("expected_slots", str(spec))
        seeds = {case["seed"] for case in spec["cases"]}
        self.assertTrue({str(-(1 << 63)), str((1 << 63) - 1)} <= seeds)
        full = next(case for case in spec["cases"] if case["id"] == "dungeon-full")
        self.assertEqual([item["slot"] for item in full["preexisting"]], list(range(27)))
        missing = next(case for case in spec["cases"] if case["id"] == "empty-fallback")
        self.assertEqual(missing["missing_table_policy"], "native_lookup")
        self.assertEqual(sum(case["size"] == 27 for case in spec["cases"]), 134)

    def test_fill_input_rejects_seed_zero_numeric_seed_and_duplicate_slots(self):
        spec, _ = fixture.input_spec("fill", None)
        for seed in ("0", 1, "9223372036854775808", "01"):
            changed = deepcopy(spec)
            changed["cases"][0]["seed"] = seed
            with self.subTest(seed=seed), self.assertRaises(ValueError):
                fixture.validate_input(changed)
        changed = deepcopy(spec)
        changed["cases"][0]["preexisting"] = [{"slot": 3, "stack": None}] * 2
        with self.assertRaisesRegex(ValueError, "duplicate"):
            fixture.validate_input(changed)

    def test_fill_input_rejects_negative_zero_luck_and_player_context(self):
        spec, _ = fixture.input_spec("fill", None)
        for luck, player in ((-0.0, None), (False, None), (0, "player"), (1, None)):
            changed = deepcopy(spec)
            changed["context"]["luck"] = luck
            changed["context"]["player"] = player
            with self.subTest(luck=luck, player=player), self.assertRaises(ValueError):
                fixture.validate_input(changed)

    def test_complete_native_export_cross_checks_keep_tag_order(self):
        values, registry = exports()
        before = deepcopy(values["items.json"]["tags"])
        self.assertEqual(data.validate_catalog(values, registry), values["loot-catalog.json"]["counts"])
        self.assertEqual(values["items.json"]["tags"], before)
        values["items.json"]["tags"][0]["members_native_order"].reverse()
        data.validate_catalog(values, registry)

    def test_historical_identity_and_stale_native_ids_are_rejected(self):
        values, registry = exports()
        values["loot-catalog.json"]["minecraft"] = "26.1"
        with self.assertRaisesRegex(ValueError, "refusing to relabel"):
            data.validate_catalog(values, registry)
        values, registry = exports()
        registry["registries"]["ITEM"][0]["key"] = "minecraft:sulfur"
        with self.assertRaisesRegex(ValueError, "registry IDs disagree"):
            data.validate_catalog(values, registry)

    def test_missing_tables_unknown_holders_and_unserialized_defaults_fail_closed(self):
        for mutation in ("table", "tag", "component", "counts"):
            values, registry = exports()
            if mutation == "table":
                values["loaded-tables.json"]["all_loaded_keys"].pop()
            elif mutation == "tag":
                values["items.json"]["tags"][0]["members_native_order"].append("minecraft:missing")
            elif mutation == "component":
                values["items.json"]["items_native_order"][0]["components"]["persistent"]["minecraft:transient_test"] = 1
            else:
                values["loot-catalog.json"]["counts"]["items"] = 3
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                data.validate_catalog(values, registry)

    def test_manifest_path_rejects_escape_and_duplicate_json_is_not_admitted(self):
        directory = loot_catalog.native.ROOT / "target/loot-test"
        for name in ("../loot.json", "C:/loot.json", "/loot.json", "sources\\loot.json", "./loot.json", ""):
            with self.subTest(name=name), self.assertRaises(ValueError):
                data.manifest_path(directory, name)
        self.assertEqual(data.manifest_path(directory, "sources/loot.json"), directory / "sources/loot.json")
        for raw in (b'{"id":1,"id":2}', b'{"id":NaN}'):
            with self.assertRaises(ValueError):
                data.decode(raw)

    def test_complete_capture_manifest_pin_and_export_integrity_are_checked(self):
        from unittest.mock import Mock

        values, registry = exports()
        directory = loot_catalog.native.ROOT / "target/mock-loot-capture"
        pin = "a" * 64
        files = {name: "b" * 64 for name in data.RUNTIME_FILES}
        files.update({name: "b" * 64 for name in (
            "capture-inputs.json", "input-spec.json", "config.json", "environment.json", "native-result.json",
            "instrumentation.json", "sources/registry_26_2.json", "sources/registry-provenance_26_2.json",
            "tools/asm-9.8.jar", "trace.jsonl")})
        files.update({"sources/" + name: expected for name, expected in data.HISTORICAL_JAVA.items()})
        manifest = {"schema": data.CAPTURE_SCHEMA, "minecraft": "26.2", "protocol": 776,
                    "jar_sha256": loot_catalog.native.JAR_SHA256, "status": "pass", "mode": "catalog",
                    "counts": values["loot-catalog.json"]["counts"], "case_count": 0, "native_fills": 0,
                    "files_sha256": files}
        spec = {"world_seed": "1", "cases": []}
        values["loot-catalog.json"]["world_seed_i64"] = "1"
        inputs = {"minecraft": "26.2", "protocol": 776, "jar_sha256": loot_catalog.native.JAR_SHA256,
                  "mode": "catalog", "historical_source_sha256": data.HISTORICAL_JAVA,
                  "source_sha256": data.HISTORICAL_JAVA, "input_spec_sha256": "b" * 64,
                  "environment_sha256": "b" * 64, "registry_sha256": "b" * 64,
                  "registry_provenance_sha256": "b" * 64, "asm_sha256": "b" * 64}
        values.update({"manifest.json": manifest, "sources/registry_26_2.json": registry,
                       "sources/registry-provenance_26_2.json": {"minecraft": "26.2", "protocol": 776,
                           "jar_sha256": loot_catalog.native.JAR_SHA256,
                           "generated_sha256": {"registries_26_2.json": "b" * 64}},
                       "native-result.json": values["loot-catalog.json"], "capture-inputs.json": inputs,
                       "instrumentation.json": {"methods": ["server/MinecraftServer.setInitialSpawn(native)",
                           "server/MinecraftServer.tickServer(native)", "server/level/ServerLevel.tick(native)"]},
                       "environment.json": {"jar_sha256": loot_catalog.native.JAR_SHA256,
                           "libraries": {str(index): "b" * 64 for index in range(39)}},
                       "input-spec.json": spec, "config.json": {**spec, "minecraft": "26.2", "protocol": 776,
                           "jar_sha256": loot_catalog.native.JAR_SHA256, "mode": "catalog"}})

        def sha(path):
            name = path.relative_to(directory).as_posix()
            return pin if name == "manifest.json" else files[name]

        with patch.object(data.native, "sha", side_effect=sha), patch.object(data, "load", side_effect=lambda path: values[path.relative_to(directory).as_posix()]), \
                patch.object(Path, "is_file", return_value=True), patch.object(Path, "exists", return_value=False), \
                patch.object(Path, "stat", return_value=Mock(st_size=0)):
            self.assertEqual(data.verified_capture(directory, pin)[0], manifest)
            with self.assertRaisesRegex(ValueError, "manifest differs"):
                data.verified_capture(directory, "c" * 64)
            with patch.object(data.native, "sha", side_effect=lambda path: "c" * 64 if path.name == "items.json" else sha(path)):
                with self.assertRaisesRegex(ValueError, "captured input changed"):
                    data.verified_capture(directory, pin)

    def test_promotion_checks_all_conflicts_before_writing_any_file(self):
        from unittest.mock import Mock

        absent, different = Mock(), Mock()
        absent.exists.return_value = False
        different.exists.return_value = True
        different.is_file.return_value = True
        different.read_bytes.return_value = b"old"
        with self.assertRaisesRegex(ValueError, "refusing to replace"):
            promotion.publish({absent: b"new", different: b"new"})
        absent.open.assert_not_called()
        absent.parent.mkdir.assert_not_called()

    def test_fixture_runtime_comparison_retains_arrays_and_ignores_only_object_key_order(self):
        root = loot_catalog.native.ROOT
        runtime = root / "crates/bcore-worldgen/data/loot_26_2"
        original = b'{"tags":["second","first"],"defaults":{"a":1,"b":2}}'
        reordered = b'{"defaults":{"b":2,"a":1},"tags":["second","first"]}'
        captured = {name: reordered for name in data.ENGINE_FILES}
        manifest = {"minecraft": "26.2", "server_sha256": loot_catalog.native.JAR_SHA256, "files": {
            (runtime / name).relative_to(root).as_posix(): {"sha256": data.digest(original)} for name in data.ENGINE_FILES}}
        with patch.object(promotion.data, "load", return_value=manifest), patch.object(Path, "read_bytes", return_value=original):
            hashes = promotion.existing_runtime_hashes(runtime, root, captured)
            self.assertEqual(set(hashes.values()), {data.digest(original)})
            captured["items.json"] = b'{"tags":["first","second"],"defaults":{"a":1,"b":2}}'
            with self.assertRaisesRegex(ValueError, "runtime context differ"):
                promotion.existing_runtime_hashes(runtime, root, captured)


if __name__ == "__main__":
    unittest.main()
