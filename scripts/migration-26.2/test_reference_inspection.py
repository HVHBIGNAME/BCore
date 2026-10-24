"""Offline identity/projection checks; no native processes or capture writes."""
from copy import deepcopy
from contextlib import redirect_stderr, redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import block_entity_replay_audit as entities
import catalog_projection as projection


class PointerTests(unittest.TestCase):
    def test_rfc6901_escape_order_empty_keys_and_numeric_object_members(self):
        data = {"": {"a/b": {"~1": {"01": [7, 11]}}}}
        self.assertIs(projection.resolve_pointer(data, ""), data)
        self.assertEqual(projection.resolve_pointer(data, "//a~1b/~01/01/1"), 11)

    def test_array_indices_and_missing_values_cannot_select_a_different_observation(self):
        data = {"rows": [13, 17], "~2": 19}
        for pointer in ("rows", "/rows/-1", "/rows/01", "/rows/2", "/rows/+1",
                        "/rows/-", "/rows/0/value", "/missing", "/~2", "/~"):
            with self.subTest(pointer=pointer), self.assertRaises(ValueError):
                projection.resolve_pointer(data, pointer)

    def test_cli_reports_full_source_digest_and_preserves_order_without_writing(self):
        raw = b'{"rows":[{"Long":9223372036854775807},{"Short":-12}]}\n'
        output = io.StringIO()
        with patch.object(Path, "read_bytes", return_value=raw), \
                patch.object(Path, "open", side_effect=AssertionError("projection attempted a write")), \
                patch.object(sys, "argv", ["catalog_projection.py", "fixture.json", "/rows"]), \
                redirect_stdout(output):
            projection.main()
        result = json.loads(output.getvalue())
        self.assertEqual(result["catalog_sha256"], hashlib.sha256(raw).hexdigest())
        self.assertEqual(result["value"], [{"Long": 9223372036854775807}, {"Short": -12}])

    def test_keys_projection_rejects_scalar_values(self):
        with patch.object(Path, "read_bytes", return_value=b'{"row":7}'), \
                patch.object(sys, "argv", ["catalog_projection.py", "fixture.json", "/row", "--keys"]), \
                redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            projection.main()
        self.assertEqual(error.exception.code, 2)


class EntityAuditTests(unittest.TestCase):
    def setUp(self):
        def state(index, name, flags=8, **properties):
            return [index, {"Name": name, "Properties": properties}, False, [], flags]

        self.fixture = {"minecraft": "26.1", "jar_sha256": entities.HISTORICAL_JAR,
                        "template_block_entities": [
                            {"state": 0, "template": "minecraft:first"},
                            {"state": 1, "template": "minecraft:second"}]}
        self.old = {"minecraft": "26.1", "jar_sha256": entities.HISTORICAL_JAR,
                    "states": [state(0, "minecraft:bed", part="head", occupied="false"),
                               state(1, "minecraft:comparator", powered="false")]}
        self.target = {"minecraft": "26.2", "jar_sha256": entities.TARGET_JAR,
                       "states": [state(0, "minecraft:comparator", powered="false"),
                                  state(1, "minecraft:bed", 0, occupied="false", part="head")],
                       "block_entities": {"minecraft:comparator": {}}}

    def test_join_uses_complete_properties_and_retains_original_row_and_new_state_id(self):
        result = entities.audit(self.fixture, self.old, self.target)
        self.assertEqual(result["rows"], 2)
        self.assertEqual(result["differences"], [{
            "row": 0, "template": "minecraft:first", "old_state": 0, "target_state": 1,
            "identity": self.target["states"][1][1], "flags": 0, "has_default": False}])

    def test_default_presence_does_not_hide_removed_native_factory_flag(self):
        self.target["block_entities"]["minecraft:bed"] = {}
        self.assertTrue(entities.audit(self.fixture, self.old, self.target)["differences"][0]["has_default"])
        self.target["states"][1][4] = 8
        self.assertEqual(entities.audit(self.fixture, self.old, self.target)["differences"], [])

    def test_missing_complete_identity_is_unknown_not_a_factory_removal(self):
        self.target["states"][1][1]["Properties"]["part"] = "foot"
        with self.assertRaisesRegex(ValueError, "identity is unavailable"):
            entities.audit(self.fixture, self.old, self.target)

    def test_duplicate_identities_and_malformed_ids_or_flags_fail_closed(self):
        for side in ("old", "target"):
            for mutation in ("duplicate", "id", "flags"):
                old, target = deepcopy(self.old), deepcopy(self.target)
                catalog = old if side == "old" else target
                if mutation == "duplicate":
                    catalog["states"][1][1] = catalog["states"][0][1]
                elif mutation == "id":
                    catalog["states"][1][0] = True
                else:
                    catalog["states"][1][4] = True
                with self.subTest(side=side, mutation=mutation), self.assertRaises(ValueError):
                    entities.audit(self.fixture, old, target)

    def test_foreign_artifact_labels_and_invalid_historical_input_ids_are_rejected(self):
        for side in ("fixture", "old", "target"):
            values = deepcopy([self.fixture, self.old, self.target])
            values[("fixture", "old", "target").index(side)]["jar_sha256"] = "0" * 64
            with self.subTest(side=side), self.assertRaises(ValueError):
                entities.audit(*values)
        for state in (-1, 2, True):
            fixture = deepcopy(self.fixture)
            fixture["template_block_entities"][0]["state"] = state
            with self.subTest(state=state), self.assertRaises(ValueError):
                entities.audit(fixture, self.old, self.target)


if __name__ == "__main__":
    unittest.main()
