"""Packet-blob admission and exact native-transcript binding regressions."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import native
import protocol_capture as capture
import protocol_data as data
import protocol_wire as wire


def blob(rows):
    return struct.pack(">I", len(rows)) + b"".join(struct.pack(">iI", packet_id, len(payload)) + payload
                                                for packet_id, payload in rows)


class PacketBlobTests(unittest.TestCase):
    def test_complete_records_preserve_empty_payload_order_and_duplicates(self):
        rows = [(7, b"registry"), (3, b""), (7, b"registry")]
        self.assertEqual(data.packets(blob(rows)), rows)
        self.assertEqual(data.packets(blob([])), [])

    def test_malformed_counts_headers_ids_lengths_and_trailing_bytes_fail(self):
        invalid = (b"", b"\0\0\0", struct.pack(">I", 65537), struct.pack(">I", 1),
                   blob([(-1, b"")]), struct.pack(">IiI", 1, 7, wire.MAX_FRAME + 1),
                   blob([(7, b"abc")])[:-1], blob([]) + b"extra")
        for raw in invalid:
            with self.subTest(raw=raw), self.assertRaises(wire.ProtocolError):
                data.packets(raw)

    def test_comparison_keeps_first_changed_record_and_count(self):
        left = blob([(7, b"one"), (7, b"two")])
        same = data.compare(left, left)
        self.assertTrue(same["equal"])
        self.assertIsNone(same["first_different_record"])
        changed = data.compare(left, blob([(7, b"one"), (7, b"other")]))
        self.assertFalse(changed["equal"])
        self.assertEqual(changed["first_different_record"], 1)
        shorter = data.compare(left, blob([(7, b"one")]))
        self.assertEqual((shorter["left_count"], shorter["right_count"], shorter["first_different_record"]), (2, 1, 1))


class ConfigurationValueTests(unittest.TestCase):
    def setUp(self):
        self.inventory = capture.PacketInventory({"minecraft": "26.2", "protocol": 776,
            "synchronized_registries": ["minecraft:test"], "packets": {"configuration": {"clientbound": {
                "minecraft:registry_data": {"protocol_id": 7}, "minecraft:finish_configuration": {"protocol_id": 3}}}}})

    def registry(self, pairs, kind=3):
        nbt = b"\x0a" + b"".join(bytes([kind]) + struct.pack(">H", len(name)) + name.encode()
                                + struct.pack(">" + {1: "b", 2: "h", 3: "i", 4: "q"}[kind], value)
                                for name, value in pairs) + b"\x00"
        return wire.string("minecraft:test") + wire.varint(1) + wire.string("minecraft:entry") + b"\x01" + nbt

    def test_compound_member_order_is_separate_from_raw_packet_identity(self):
        first = blob([(7, self.registry([("one", 1), ("two", 2)])), (3, b"")])
        reordered = blob([(7, self.registry([("two", 2), ("one", 1)])), (3, b"")])
        self.assertFalse(data.compare(first, reordered)["equal"])
        self.assertTrue(data.compare_configuration(first, reordered, self.inventory)["equal"])

    def test_value_and_packet_order_changes_are_not_normalized(self):
        first = blob([(7, self.registry([("one", 1)])), (3, b"")])
        changed = blob([(7, self.registry([("one", 2)])), (3, b"")])
        self.assertEqual(data.compare_configuration(first, changed, self.inventory)["changed_records"], [0])
        reordered = blob([(3, b""), (7, self.registry([("one", 1)]))])
        self.assertFalse(data.compare_configuration(first, reordered, self.inventory)["equal"])

    def test_missing_registry_data_and_nonempty_finish_are_rejected(self):
        absent = wire.string("minecraft:test") + wire.varint(1) + wire.string("minecraft:entry") + b"\x00"
        for raw in (blob([(7, absent)]), blob([(3, b"extra")])):
            with self.subTest(raw=raw), self.assertRaises(wire.ProtocolError):
                data.configuration_values(raw, self.inventory)

    def test_numeric_tag_width_is_part_of_the_configuration_contract(self):
        byte = blob([(7, self.registry([("value", 1)], kind=1))])
        integer = blob([(7, self.registry([("value", 1)], kind=3))])
        self.assertFalse(data.compare_configuration(byte, integer, self.inventory)["equal"])

    def test_nbt_list_order_and_signed_zero_bits_are_not_normalized(self):
        prefix = wire.string("minecraft:test") + b"\x01" + wire.string("minecraft:entry") + b"\x01\x0a"
        def payload(values, bits):
            return prefix + b"\x09\x00\x04list\x03" + struct.pack(">i2i", 2, *values) + b"\x05\x00\x04zero" + struct.pack(">I", bits) + b"\x00"
        first = blob([(7, payload([1, 2], 0))])
        for other in (payload([2, 1], 0), payload([1, 2], 0x80000000)):
            with self.subTest(other=other):
                self.assertFalse(data.compare_configuration(first, blob([(7, other)]), self.inventory)["equal"])


class NativeProofTests(unittest.TestCase):
    def setUp(self):
        self.root = (native.ROOT / "target/offline-proof").resolve()
        self.source = (native.ROOT / "target/offline-source").resolve()
        self.session = self.source / "session-1"
        self.result = dict.fromkeys(("status", "schema_validation", "native_session_validation", "source_input_freeze"), "pass")
        self.result.update(mode="official", cleanup=[{"state": "clean", "forced": False, "log_errors": []}])
        self.receipt = {"manifest": {"sha256": "manifest"}, "results": {"sha256": "results"}}
        self.inputs = {"original_capture": {"path": str(self.source), **copy.deepcopy(self.receipt)}}

    def proof(self, source=None):
        source = self.root if source is None else source
        docs = {self.root / "manifest.json": {}, self.root / "results.json": self.result}
        with patch.object(Path, "read_bytes", autospec=True,
                          side_effect=lambda path: json.dumps(docs[path]).encode()), \
                patch.object(data, "checked_file", side_effect=lambda root, name, manifest:
                             json.dumps(self.inputs if name == "inputs.json" else {}).encode()), \
                patch.object(capture, "check_schema_inputs", return_value=[{"decoded": 2}]), \
                patch.object(capture, "identity", return_value={"sha256": "observed"}):
            return data.native_proof(self.root, source, [self.session], self.receipt)

    def test_official_validation_requires_every_gate_and_clean_owned_shutdown(self):
        self.assertEqual(self.proof()["coverage"], [{"decoded": 2}])
        baseline = copy.deepcopy(self.result)
        for key in ("status", "schema_validation", "native_session_validation", "source_input_freeze"):
            self.result = {**baseline, key: "partial"}
            with self.subTest(key=key), self.assertRaises(wire.ProtocolError):
                self.proof()
        for cleanup in ([], [{"state": "clean", "forced": True}], [{"state": "unknown", "forced": False}]):
            self.result = {**baseline, "cleanup": cleanup}
            with self.subTest(cleanup=cleanup), self.assertRaises(wire.ProtocolError):
                self.proof()

    def test_reverify_requires_exact_original_capture_and_unchanged_inputs(self):
        self.result.update(operation="reverify_recorded_protocol", recorded_input_freeze="pass")
        self.assertEqual(self.proof(self.source)["coverage"], [{"decoded": 2}])
        self.inputs["original_capture"]["manifest"]["sha256"] = "different"
        with self.assertRaisesRegex(wire.ProtocolError, "not bound"):
            self.proof(self.source)
        self.inputs["original_capture"]["manifest"]["sha256"] = "manifest"
        self.result["recorded_input_freeze"] = "fail"
        with self.assertRaisesRegex(wire.ProtocolError, "not bound"):
            self.proof(self.source)


class BCoreAdmissionTests(unittest.TestCase):
    def result(self):
        value = dict.fromkeys(("status", "schema_validation", "native_session_validation", "source_input_freeze"), "pass")
        value.update(mode="bcore", bcore_executable_freeze={"status": "pass"},
                     persistence_probe_freeze={"status": "pass"}, persistence={"status": "pass"})
        cleanup = {"state": "clean", "forced": False, "root_exit_code": 0, "job_active_before_close": 0,
                   "listeners_after": [], "errors": [], "log_errors": [], "port_exclusively_rebound": True}
        value["cleanup"] = [copy.deepcopy(cleanup), copy.deepcopy(cleanup)]
        for key in ("shutdown", "shutdown_after_reconnect"):
            value[key] = {"status": "pass", "root_exit_code": 0, "method": "operator /stop"}
        return value

    def test_full_native_and_persisted_edit_proof_are_required(self):
        inputs = {"options": {"mode": "bcore", "exercise": "full"}}
        baseline = self.result()
        data.require_bcore_result(baseline, inputs)
        for key in ("status", "schema_validation", "native_session_validation", "source_input_freeze"):
            with self.subTest(key=key), self.assertRaises(wire.ProtocolError):
                data.require_bcore_result({**baseline, key: "partial"}, inputs)
        for key in ("bcore_executable_freeze", "persistence_probe_freeze", "persistence"):
            with self.subTest(key=key), self.assertRaises(wire.ProtocolError):
                data.require_bcore_result({**baseline, key: {"status": "unknown"}}, inputs)

    def test_forced_cleanup_retained_listener_or_missing_restart_cannot_publish(self):
        inputs = {"options": {"mode": "bcore", "exercise": "full"}}
        for change in ({"forced": True}, {"listeners_after": [{"pid": 12}]}, {"root_exit_code": 1},
                       {"job_active_before_close": 1}, {"port_exclusively_rebound": False}):
            value = self.result()
            value["cleanup"][1].update(change)
            with self.subTest(change=change), self.assertRaises(wire.ProtocolError):
                data.require_bcore_result(value, inputs)
        value = self.result()
        del value["shutdown_after_reconnect"]
        with self.assertRaises(wire.ProtocolError):
            data.require_bcore_result(value, inputs)


class PublicationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(dir=native.ROOT / "target")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.directory = self.root / "crates/bcore-protocol/data"
        self.metrics = self.root / "docs/metrics"
        self.directory.mkdir(parents=True)
        self.metrics.mkdir(parents=True)
        for name in data.FILES.values():
            (self.directory / name).write_bytes(blob([(1, b"synthetic packet")]))
        self.original = self.directory / "protocol_26_2.json"
        self.original.write_bytes(b"retained original publication")
        for target, attribute, value in ((data, "DATA", self.directory), (native, "ROOT", self.root)):
            mocked = patch.object(target, attribute, value)
            mocked.start()
            self.addCleanup(mocked.stop)
        self.result = {
            "published_configuration_values": {"equal": True},
            "repeat_configuration_values": {"equal": True},
            "registry_bindings": {"biome_ids_match": True},
            "bcore_validation": {"synthetic": True},
            "source": {"capture": {"manifest": {"sha256": "source"}}},
            "repeat": {"capture": {"manifest": {"sha256": "repeat"}}},
        }

    def test_versioned_metrics_publication_preserves_the_original(self):
        destination = self.metrics / "protocol-26_2-validation.json"
        identity = data.publish_bootstrap(self.result, destination)
        self.assertEqual(identity["sha256"], native.sha(destination))
        self.assertEqual(json.loads(destination.read_bytes())["schema"], "bcore.protocol-bootstrap.v1")
        self.assertEqual(self.original.read_bytes(), b"retained original publication")
        self.assertEqual(data.publish_bootstrap(self.result, destination), identity)
        self.result["bcore_validation"]["synthetic"] = "different"
        with self.assertRaisesRegex(wire.ProtocolError, "different bootstrap publication"):
            data.publish_bootstrap(self.result, destination)
        self.assertEqual(native.sha(destination), identity["sha256"])

    def test_unsupported_destination_and_incomplete_proof_do_not_write(self):
        for destination in (self.root / "outside.json", self.metrics / "not-json.txt"):
            with self.subTest(destination=destination), self.assertRaises(wire.ProtocolError):
                data.publish_bootstrap(self.result, destination)
            self.assertFalse(destination.exists())
        destination = self.metrics / "incomplete.json"
        self.result["repeat_configuration_values"]["equal"] = False
        with self.assertRaisesRegex(wire.ProtocolError, "both native configuration captures"):
            data.publish_bootstrap(self.result, destination)
        self.assertFalse(destination.exists())


if __name__ == "__main__":
    unittest.main()
