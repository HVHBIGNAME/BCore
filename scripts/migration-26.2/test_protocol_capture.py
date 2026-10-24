"""Offline protocol fixtures: no JVM, Cargo, queue, TCP connections or installs."""
from contextlib import redirect_stderr
import io
import json
from pathlib import Path
import struct
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, call, patch
import uuid

import native
import protocol_capture as capture
import protocol_runtime as runtime
import protocol_wire as wire


def nbt_name(text: str) -> bytes:
    encoded = text.encode("utf-8")
    return struct.pack(">H", len(encoded)) + encoded


def login_payload() -> bytes:
    return bytes(range(16)) + wire.string("BCore262Probe") + wire.varint(1) + wire.string("textures") + wire.string("value") \
        + b"\x01" + wire.string("signature") + uuid.UUID("6d262776-971b-4d53-8081-51f32e8c1234").bytes


def play_login_payload(dimension_type: int = 0) -> bytes:
    return (struct.pack(">i", 392) + b"\x00" + wire.varint(1) + wire.string("minecraft:overworld")
            + b"\x04\x02\x02\x00\x01\x00" + wire.varint(dimension_type) + wire.string("minecraft:overworld")
            + struct.pack(">q", capture.hashed_seed(capture.DEFAULT_SEED))
            + b"\x01\xff\x00\x01\x00\x00\x3f\x00\x00")


def flat_chunk() -> bytes:
    heightmaps = wire.varint(3)
    packed = sum(4 << (index * 9) for index in range(7))
    longs = [packed] * 36 + [sum(4 << (index * 9) for index in range(4))]
    for kind in (1, 4, 5):
        heightmaps += wire.varint(kind) + wire.varint(37) + struct.pack(">37Q", *longs)
    sections = b""
    for index in range(24):
        sections += struct.pack(">hh", 4096 if index == 0 else 0, 0)
        sections += b"\x00" + wire.varint(1 if index == 0 else 0) + b"\x00" + wire.varint(40)
    return struct.pack(">ii", -2, 3) + heightmaps + wire.varint(len(sections)) + sections + bytes(7)


class WireTests(unittest.TestCase):
    def test_signed_varints_and_split_frames(self):
        for value in (0, 127, 128, 776, -1, -(1 << 31), (1 << 31) - 1):
            with self.subTest(value=value):
                reader = wire.Reader(wire.varint(value))
                self.assertEqual(reader.varint(), value)
                reader.finish()
        raw = wire.frame(0x48, b"x" * 500)
        for index in range(len(raw)):
            self.assertIsNone(wire.split_frame(raw[:index]))
        self.assertEqual(wire.split_frame(raw + b"next"), (len(raw), 0x48, b"x" * 500))

    def test_bad_varints_lengths_and_utf8_are_rejected(self):
        for raw in (b"\x80\x00", b"\xff\xff\xff\xff\x10", b"\x80" * 5):
            with self.subTest(raw=raw), self.assertRaises(wire.ProtocolError):
                wire.Reader(raw).varint()
        for raw in (b"\x00", b"\x80\x00", b"\x80\x80\x80"):
            with self.subTest(raw=raw), self.assertRaises(wire.ProtocolError):
                wire.split_frame(raw)
        with self.assertRaises(wire.ProtocolError):
            wire.Reader(b"\x01\xff").string()
        with self.assertRaises(wire.ProtocolError):
            wire.Reader(b"\x02").boolean()

    def test_login_requires_entire_profile_and_session_uuid(self):
        raw = login_payload()
        decoded = wire.login_finished(raw)
        self.assertEqual(decoded["sessionUUID"], "6d262776-971b-4d53-8081-51f32e8c1234")
        self.assertEqual(decoded["properties"], [{"name": "textures", "value": "value", "signature": "signature"}])
        for malformed in (raw[:-1], raw + b"\x00", raw[:-16]):
            with self.subTest(length=len(malformed)), self.assertRaises(wire.ProtocolError):
                wire.login_finished(malformed)

    def test_nil_session_uuid_is_a_complete_uuid(self):
        raw = login_payload()[:-16] + bytes(16)
        self.assertEqual(wire.login_finished(raw)["sessionUUID"], str(uuid.UUID(int=0)))
        with self.assertRaises(wire.ProtocolError):
            wire.login_finished(raw[:-1])

    def test_play_login_requires_both_new_booleans(self):
        raw = play_login_payload()
        decoded = wire.play_login(raw)
        self.assertFalse(decoded["onlineMode"])
        self.assertFalse(decoded["enforcesSecureChat"])
        self.assertEqual(decoded["hashed_seed"], -1328540181389140506)
        for malformed in (raw[:-1], raw + b"\x00", raw[:-2] + b"\x02\x00"):
            with self.subTest(raw=malformed), self.assertRaises(wire.ProtocolError):
                wire.play_login(malformed)

    def test_dimension_holder_is_raw_registry_varint_including_zero(self):
        for index in (0, 1, 127, 128):
            with self.subTest(index=index):
                decoded = wire.play_login(play_login_payload(index))
                self.assertEqual(decoded["dimension_type"], index)
                self.assertEqual(decoded["dimension"], "minecraft:overworld")
                self.assertEqual(decoded["sea_level"], 63)
        with self.assertRaises(wire.ProtocolError):
            wire.play_login(play_login_payload(-1))

    def test_nbt_widths_empty_lists_modified_utf8_and_full_registry(self):
        data = (b"\x0a" + b"\x04" + nbt_name("seed") + struct.pack(">q", (1 << 63) - 1)
                + b"\x09" + nbt_name("empty") + b"\x0a" + struct.pack(">i", 0)
                + b"\x08" + nbt_name("text") + b"\x00\x08a\xc0\x80\xed\xa0\xbd\xed\xb8\x80" + b"\x00")
        # Modified UTF-8 supplementary characters are encoded as two UTF-16 units.
        data = data.replace(b"\x00\x08a", b"\x00\x09a")
        raw = wire.string("minecraft:test") + b"\x01" + wire.string("minecraft:entry") + b"\x01" + data
        decoded = wire.registry_data(raw)
        fields = decoded["entries"][0]["data"]["Compound"]
        self.assertEqual(fields["seed"], {"Long": (1 << 63) - 1})
        self.assertEqual(fields["empty"], {"List": {"element_type": 10, "values": []}})
        self.assertEqual(fields["text"], {"String": "a\0😀"})
        for malformed in (raw + b"\x00", raw[:-1], wire.string("minecraft:test") + b"\x01" + wire.string("minecraft:entry") + b"\x00"):
            with self.subTest(length=len(malformed)), self.assertRaises(wire.ProtocolError):
                wire.registry_data(malformed)

    def test_cookie_absent_is_not_present_empty(self):
        prefix = wire.string("minecraft:cookie")
        self.assertEqual(wire.cookie_response("minecraft:cookie", None), prefix + b"\x00")
        self.assertEqual(wire.cookie_response("minecraft:cookie", b""), prefix + b"\x01\x00")
        self.assertEqual(wire.store_cookie(prefix + b"\x03abc"), ("minecraft:cookie", b"abc"))
        with self.assertRaises(wire.ProtocolError):
            wire.cookie_response("minecraft:cookie", bytes(5121))
        with self.assertRaises(wire.ProtocolError):
            wire.cookie_key(prefix + b"\x00")

    def test_full_chunk_sections_and_light_masks(self):
        raw = flat_chunk()
        decoded = wire.chunk(raw, height=384, block_states=32366, biomes=66)
        self.assertEqual((decoded["x"], decoded["z"]), (-2, 3))
        self.assertEqual(len(decoded["sections"]), 24)
        self.assertEqual(decoded["sections"][0]["block_states"], {1: 4096})
        for malformed in (raw[:-1], raw + b"\x00"):
            with self.subTest(length=len(malformed)), self.assertRaises(wire.ProtocolError):
                wire.chunk(malformed, height=384, block_states=32366, biomes=66)
        with self.assertRaises(wire.ProtocolError):
            wire.palette(wire.Reader(b"\x00" + wire.varint(66)), 64, 66, 3, 1)

    def test_native_player_info_enum_order_is_list_order_then_hat(self):
        raw = b"\xc0\x01" + bytes(16) + wire.varint(7) + b"\x01"
        self.assertEqual(wire.player_info(raw)["entries"][0]["list_order"], 7)
        with self.assertRaises(wire.ProtocolError):
            wire.player_info(b"\xc0\x01" + bytes(16) + b"\x01\x07")


class BoundaryTests(unittest.TestCase):
    def setUp(self):
        self.environment = patch.object(native, "environment", side_effect=AssertionError("native environment invoked"))
        self.launch = patch.object(capture, "OwnedProcess", side_effect=AssertionError("owned process launched"))
        self.environment.start()
        self.launch.start()
        self.addCleanup(self.environment.stop)
        self.addCleanup(self.launch.stop)

    def test_invalid_cli_rejected_before_authentication_or_launch(self):
        for arguments in (["official", "--output", str(capture.HERE)],
                          ["official", "--output", str(native.ROOT / "outside")],
                          ["official", "--output", str(native.ROOT / "target/new-fixture"), "--radius", "32"],
                          ["bcore", "--output", str(native.ROOT / "target/new-fixture")]):
            with self.subTest(arguments=arguments), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as result:
                capture.parse_args(arguments)
            self.assertEqual(result.exception.code, 2)

    def test_official_properties_are_isolated_and_bounded(self):
        properties = capture.server_properties(capture.DEFAULT_SEED, 49152, "flat")
        self.assertEqual(properties["online-mode"], "false")
        self.assertEqual(properties["network-compression-threshold"], "-1")
        self.assertEqual(properties["server-ip"], "127.0.0.1")
        self.assertEqual(properties["management-server-enabled"], "false")
        self.assertEqual(properties["view-distance"], "2")
        self.assertEqual(json.loads(properties["generator-settings"])["structure_overrides"], [])
        self.assertEqual(capture.parse_properties(b"# comment\nlevel-type=minecraft\\:flat\n"), {"level-type": "minecraft:flat"})

    def test_status_retry_settle_cli_is_short_and_finite(self):
        arguments = ["official", "--output", str(native.ROOT / "target/new-fixture")]
        self.assertEqual(capture.parse_args(arguments).status_retry_settle, 0.5)
        for seconds in ("0", "-1", "5.01", "nan", "inf"):
            with self.subTest(seconds=seconds), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                capture.parse_args([*arguments, "--status-retry-settle", seconds])

    def test_reverify_cli_selects_immutable_capture_and_new_output(self):
        arguments = ["reverify", "--output", str(native.ROOT / "target/new-schema-result")]
        root = native.ROOT / "target/offline-capture"
        with patch.object(Path, "resolve", autospec=True, side_effect=lambda path, strict=False: path), \
                patch.object(Path, "is_dir", return_value=True):
            parsed = capture.parse_args([*arguments, "--capture", str(root)])
        self.assertEqual(parsed.capture, root)
        for invalid in (arguments, [*arguments, "--capture", str(native.ROOT)],
                        [*arguments, "--capture", str(root), "--executable", "server.exe"],
                        ["official", "--output", str(root / "new-schema-result"), "--capture", str(root)]):
            with self.subTest(arguments=invalid), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                capture.parse_args(invalid)
        with patch.object(capture, "exclusive_output", return_value=root / "inside-capture"), \
                patch.object(Path, "resolve", autospec=True, side_effect=lambda path, strict=False: path), \
                patch.object(Path, "is_dir", return_value=True), \
                redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            capture.parse_args([*arguments, "--capture", str(root)])

    def session_fixture(self):
        # Construct an in-memory Session without a Capture directory or socket.
        session = capture.Session.__new__(capture.Session)
        session.args = Mock(chunks_per_tick=2.0, max_chunks=128, seed=capture.DEFAULT_SEED)
        session.socket = Mock(state="play")
        session.inventory = Mock()
        session.seen, session.batch, session.teleport = set(), None, None
        session.loaded, session.bundle_open = False, False
        session.cookies = {}
        session.play = {"checks": [], "batches": [], "chunks": [], "state_packets": {}}
        session.player_list, session.cached_chunks = {}, set()
        return session

    def test_control_replies_have_exact_payload_types(self):
        session = self.session_fixture()
        self.assertTrue(session.common("keep_alive", struct.pack(">q", 0x1234)))
        session.inventory.send.assert_called_with(session.socket, "keep_alive", struct.pack(">q", 0x1234))
        self.assertTrue(session.common("ping", struct.pack(">i", -10)))
        session.inventory.send.assert_called_with(session.socket, "pong", struct.pack(">i", -10))
        with self.assertRaises(wire.ProtocolError):
            session.common("keep_alive", b"\x01")
        session.common("cookie_request", wire.string("minecraft:unknown"))
        session.inventory.send.assert_called_with(session.socket, "cookie_response", wire.string("minecraft:unknown") + b"\x00")

    def test_teleport_echo_and_batch_float_ack_loaded(self):
        session = self.session_fixture()
        teleport = wire.varint(300) + struct.pack(">6d2fI", 0.5, 80, -0.5, 0, 0, 0, 0, 0, 0)
        session.socket.receive.return_value = (0x48, teleport, {"sequence": 1})
        session.inventory.name.return_value = "player_position"
        session.receive_play(100)
        session.inventory.send.assert_called_with(session.socket, "accept_teleportation", wire.varint(300))
        session.batch = [[0, -1]]
        session.socket.receive.return_value = (0x0b, wire.varint(1), {"sequence": 2})
        session.inventory.name.return_value = "chunk_batch_finished"
        session.receive_play(100)
        sends = [call.args[1:] for call in session.inventory.send.call_args_list]
        self.assertIn(("chunk_batch_received", struct.pack(">f", 2.0)), sends)
        self.assertIn(("player_loaded",), sends)
        self.assertTrue(session.loaded)

    def test_mismatched_batch_is_failure_without_ack(self):
        session = self.session_fixture()
        session.batch = [[0, 0]]
        session.socket.receive.return_value = (0x0b, wire.varint(2), {"sequence": 1})
        session.inventory.name.return_value = "chunk_batch_finished"
        with self.assertRaises(wire.ProtocolError):
            session.receive_play(100)
        session.inventory.send.assert_not_called()

    def test_fragmented_receive_uses_exact_frame_boundary(self):
        connection = capture.Connection.__new__(capture.Connection)
        connection.socket, connection.capture, connection.owner = Mock(), Mock(), Mock()
        connection.state, connection.timeout, connection.buffer = "play", 1, bytearray()
        connection.outcome = {"connection": 0, "received_bytes": 0}
        connection.capture.record.return_value = {}
        raw = wire.frame(0x2c, struct.pack(">q", 123))
        chunks = iter([raw[:1], raw[1:2], raw[2:5], raw[5:]])
        connection.socket.recv.side_effect = lambda length: next(chunks)
        packet_id, payload, _ = connection.receive(capture.time.monotonic() + 5)
        self.assertEqual((packet_id, payload), (0x2c, struct.pack(">q", 123)))
        self.assertEqual(connection.buffer, b"")


class MemoryCapture:
    def __init__(self):
        self.packet_limit, self.byte_limit = 100, 65536
        self.events, self.sockets = [], []
        self.total_bytes = 0

    def record(self, state, direction, packet_id, payload, framed):
        event = {"sequence": len(self.events), "state": state, "direction": direction,
                 "packet_id": packet_id, "frame_sha256": capture.digest(framed), "raw": framed}
        self.events.append(event)
        self.total_bytes += len(framed)
        return event


class StatusTests(unittest.TestCase):
    def setUp(self):
        self.capture = MemoryCapture()
        self.owner = Mock(process=Mock(pid=123))
        self.session = capture.Session.__new__(capture.Session)
        self.session.args = SimpleNamespace(socket_timeout=0.1)
        self.session.owner, self.session.port, self.session.capture = self.owner, 51132, self.capture
        self.session.connections = []
        self.session.status_probe = {"status": "not_attempted", "attempts": [], "max_attempts": 2,
                                     "retry_settle_seconds": 0.5}
        self.listener = patch.object(capture, "require_listener", return_value={"listener": {"pid": 123}})
        self.listener.start()
        self.addCleanup(self.listener.stop)
        self.socket_patch = patch.object(capture.socket, "socket")
        self.socket_factory = self.socket_patch.start()
        self.addCleanup(self.socket_patch.stop)
        self.settle_patch = patch.object(capture, "settle_listener", return_value={"requested_seconds": 0.5})
        self.settle = self.settle_patch.start()
        self.addCleanup(self.settle_patch.stop)

    def socket(self, first_read):
        sock = Mock()
        sock.getsockname.return_value = ("127.0.0.1", 60000)
        sock.getpeername.return_value = ("127.0.0.1", self.session.port)
        sock.recv.side_effect = first_read
        return sock

    def responding_socket(self, version=None, pong=None):
        result = {"version": version or {"name": "26.2", "protocol": 776}}
        pending = bytearray(wire.frame(0, wire.string(json.dumps(result))))
        sock = self.socket(None)

        def receive(size):
            data = bytes(pending[:size])
            del pending[:size]
            return data

        def send(framed):
            _, packet_id, payload = wire.split_frame(framed)
            if packet_id == 1:
                if isinstance(pong, BaseException):
                    sock.recv.side_effect = pong
                else:
                    pending.extend(wire.frame(1, payload if pong is None else pong))

        sock.recv.side_effect = receive
        sock.sendall.side_effect = send
        return sock, result

    def test_first_status_eof_retained_with_exact_retry(self):
        failed = self.socket([b""])
        successful, expected = self.responding_socket()
        self.socket_factory.side_effect = [failed, successful]
        self.assertEqual(self.session.status(), expected)
        probe = self.session.status_probe
        self.assertEqual((probe["status"], probe["initial_status"], probe["successful_attempt"]),
                         ("pass_after_retry", "fail", 2))
        first, second = probe["attempts"]
        self.assertEqual(first["error"], "EOFError: peer closed during status")
        self.assertEqual(first["socket_end"], "peer_eof")
        self.assertEqual(first["handshake_frame_sha256"], second["handshake_frame_sha256"])
        self.assertEqual(first["request_frame_sha256"], second["request_frame_sha256"])
        self.assertEqual(failed.sendall.call_args_list, successful.sendall.call_args_list[:2])
        self.assertEqual([event["connection"] for event in self.capture.events], [0, 0, 1, 1, 1, 1, 1])
        self.settle.assert_called_once_with(self.owner, 51132, 0.5)
        failed.close.assert_called_once()
        successful.close.assert_called_once()

    def test_first_success_needs_no_retry(self):
        sock, expected = self.responding_socket()
        self.socket_factory.return_value = sock
        self.assertEqual(self.session.status(), expected)
        self.assertEqual(self.session.status_probe["status"], "pass")
        self.assertEqual(len(self.session.status_probe["attempts"]), 1)
        self.settle.assert_not_called()

    def test_second_eof_exhausts_budget_and_stays_failed(self):
        self.socket_factory.side_effect = [self.socket([b""]), self.socket([b""])]
        with self.assertRaises(EOFError):
            self.session.status()
        self.assertEqual(self.session.status_probe["status"], "fail")
        self.assertEqual([row["status"] for row in self.session.status_probe["attempts"]], ["fail", "fail"])
        self.assertEqual(self.socket_factory.call_count, 2)
        self.settle.assert_called_once()

    def test_bcore_status_failure_is_not_retried(self):
        self.session.status_probe["max_attempts"] = 1
        self.socket_factory.return_value = self.socket([b""])
        with self.assertRaises(EOFError):
            self.session.status()
        self.settle.assert_not_called()

    def test_partial_response_timeout_is_not_retried(self):
        self.socket_factory.return_value = self.socket([b"\x80", TimeoutError("partial status")])
        with patch.object(capture.time, "monotonic", side_effect=(0, 0.01, 0.02, 0.2)), self.assertRaises(TimeoutError):
            self.session.status()
        self.assertEqual(self.session.status_probe["attempts"][0]["partial_frame_hex"], "80")
        self.settle.assert_not_called()

    def test_empty_response_timeout_can_retry(self):
        failed = self.socket(TimeoutError("status pending"))
        successful, expected = self.responding_socket()
        self.socket_factory.side_effect = [failed, successful]
        with patch.object(capture.time, "monotonic", side_effect=(0, 0.01, 0.2, 1, 1.01, 1.02, 1.03, 1.04, 1.05, 1.06)):
            self.assertEqual(self.session.status(), expected)
        self.assertEqual(self.session.status_probe["status"], "pass_after_retry")
        self.assertEqual(self.session.status_probe["attempts"][0]["received_bytes"], 0)
        self.settle.assert_called_once()

    def test_partial_eof_is_a_protocol_failure_without_retry(self):
        self.socket_factory.return_value = self.socket([b"\x02", b""])
        with self.assertRaisesRegex(wire.ProtocolError, "EOF inside status frame"):
            self.session.status()
        self.assertEqual(self.session.status_probe["attempts"][0]["partial_frame_hex"], "02")
        self.settle.assert_not_called()

    def test_wrong_version_and_ping_failures_are_not_retried(self):
        for version, pong, error in (({"name": "26.1", "protocol": 775}, None, wire.ProtocolError),
                                     (None, b"bad", wire.ProtocolError),
                                     (None, EOFError("ping EOF"), EOFError)):
            with self.subTest(version=version, pong=pong):
                self.session.status_probe["attempts"] = []
                sock, _ = self.responding_socket(version, pong)
                self.socket_factory.return_value = sock
                with self.assertRaises(error):
                    self.session.status()
                self.assertEqual(self.session.status_probe["status"], "fail")
                self.assertEqual(len(self.session.status_probe["attempts"]), 1)
        self.settle.assert_not_called()

    def test_dead_owner_during_settle_prevents_retry(self):
        self.socket_factory.return_value = self.socket([b""])
        self.settle.side_effect = RuntimeError("server exited early: 0")
        with self.assertRaisesRegex(RuntimeError, "server exited early"):
            self.session.status()
        self.assertEqual(self.socket_factory.call_count, 1)
        self.assertIn("server exited early", self.session.status_probe["retry_error"])

    def test_status_failure_does_not_mark_login_or_play_as_attempted(self):
        session = capture.Session.__new__(capture.Session)
        session.phase, session.connections, session.gates = "status", [], None
        session.capture = Mock(output=Path("memory-session"))
        session.auth = {"status": "not_attempted"}
        session.config = {"status": "not_attempted"}
        session.play = {"status": "not_attempted"}
        session.status_probe = {"status": "fail"}
        with patch.object(native, "write_json") as write:
            session.finish(EOFError("status"))
        for phase in (session.auth, session.config, session.play):
            self.assertEqual(phase["status"], "not_attempted")
        self.assertIn(call(Path("memory-session/status.json"), {"status": "fail"}), write.call_args_list)
        self.assertEqual(session.capture.close.call_args.args[0]["failed_phase"], "status")


class ReadinessTests(unittest.TestCase):
    def setUp(self):
        self.now = 0.0
        self.owner = Mock(process=Mock(pid=123))
        self.row = {"host": "127.0.0.1", "port": 51132, "pid": 123}
        self.clock = patch.object(runtime.time, "monotonic", side_effect=lambda: self.now)
        self.sleep = patch.object(runtime.time, "sleep", side_effect=self.advance)
        self.clock.start()
        self.sleep.start()
        self.addCleanup(self.clock.stop)
        self.addCleanup(self.sleep.stop)

    def advance(self, seconds):
        self.now += seconds

    def test_marker_and_listener_do_not_claim_status_available(self):
        self.owner.stdout_tail.side_effect = [b"Starting", b"Done (0.563s)! For help, type help\n"]
        with patch.object(runtime, "listeners", return_value=[self.row]):
            result = runtime.wait_ready(self.owner, 51132, 1, native_server=True)
        self.assertEqual(result["status_availability"], "not_probed")
        self.assertEqual(result["readiness_scope"], "startup_marker_and_owned_listener")
        self.assertAlmostEqual(self.now, 0.1)

    def test_owned_listener_settle_is_bounded_and_rechecked(self):
        with patch.object(runtime, "listeners", return_value=[self.row]):
            result = runtime.settle_listener(self.owner, 51132, 0.5)
        self.assertAlmostEqual(self.now, 0.5)
        self.assertEqual(result["ownership_before"]["listener"], result["ownership_after"]["listener"])
        self.assertGreater(self.owner.check.call_count, 2)

    def test_dead_process_aborts_settle_before_delay(self):
        self.owner.check.side_effect = RuntimeError("server exited")
        with self.assertRaisesRegex(RuntimeError, "server exited"):
            runtime.settle_listener(self.owner, 51132, 0.5)
        self.assertEqual(self.now, 0)

    def test_settle_invalid_duration_never_probes_listener(self):
        for seconds in (0, -1, 5.01, float("nan"), float("inf")):
            with self.subTest(seconds=seconds), self.assertRaises(ValueError):
                runtime.settle_listener(self.owner, 51132, seconds)
        self.owner.check.assert_not_called()

    def test_listener_replacement_during_settle_fails(self):
        other = {**self.row, "pid": 456}
        with patch.object(runtime, "listeners", side_effect=[[self.row], [other]]), self.assertRaises(RuntimeError):
            runtime.settle_listener(self.owner, 51132, 0.5)


class SchemaCoverageTests(unittest.TestCase):
    def report(self, coverage):
        path = native.ROOT / "target/session-offline"
        row = {"status": "pass", "decoded": 2, "expected": 2, "capture_path": str(path), "coverage": coverage,
               "packets": [{"sequence": 0, "status": "pass"}, {"sequence": 1, "status": "pass"}]}
        return {"minecraft": "26.2", "protocol": 776, "sessions": [row]}, [path]

    def test_two_outbound_frames_pass_decode_only_not_session_gate(self):
        report, paths = self.report({"states": {"handshake:serverbound": 1, "status:serverbound": 1},
                                     "login_finished": False, "native_terrain_packets": 0})
        rows = capture.check_schema_report(report, paths)
        self.assertEqual(rows[0]["scope"], "recorded_frames_only")
        with self.assertRaisesRegex(wire.ProtocolError, "full-session coverage incomplete"):
            capture.check_schema_report(report, paths, min_chunks=1)

    def test_full_schema_gate_requires_observed_login_registry_and_terrain(self):
        coverage = dict.fromkeys(("status_response", "status_pong", "login_finished", "login_acknowledged",
                                  "network_registry_load", "configuration_acknowledged", "play_login"), True)
        coverage.update(states={"play:clientbound": 2}, native_terrain_packets=1)
        report, paths = self.report(coverage)
        self.assertEqual(len(capture.check_schema_report(report, paths, min_chunks=1)), 1)
        for missing in ("status_response", "status_pong", "login_finished", "login_acknowledged",
                        "network_registry_load", "configuration_acknowledged", "play_login"):
            with self.subTest(missing=missing):
                report["sessions"][0]["coverage"] = {**coverage, missing: False}
                with self.assertRaises(wire.ProtocolError):
                    capture.check_schema_report(report, paths, min_chunks=1)
        report["sessions"][0]["coverage"] = {**coverage, "native_terrain_packets": 0}
        with self.assertRaises(wire.ProtocolError):
            capture.check_schema_report(report, paths, min_chunks=1)

    def test_empty_or_missing_coverage_reports_fail_closed(self):
        report, paths = self.report(None)
        with self.assertRaisesRegex(wire.ProtocolError, "coverage missing"):
            capture.check_schema_report(report, paths)
        report["sessions"] = []
        with self.assertRaises(wire.ProtocolError):
            capture.check_schema_report(report, [])

    def test_failed_native_registry_record_is_exposed_even_if_summary_says_pass(self):
        report, paths = self.report({})
        report["sessions"][0]["packets"][1] = {
            "sequence": 46, "state": "configuration", "direction": "clientbound",
            "packet": "minecraft:finish_configuration", "status": "fail", "failure_phase": "network_registry_load",
            "error": "Missing tag: minecraft:infiniburn_overworld"}
        with self.assertRaisesRegex(wire.ProtocolError, "sequence 46 configuration/clientbound.*network_registry_load.*Missing tag"):
            capture.check_schema_report(report, paths)

    def test_all_frames_are_required_even_with_passing_aggregate(self):
        report, paths = self.report({})
        report["sessions"][0]["packets"].pop()
        with self.assertRaisesRegex(wire.ProtocolError, "packet report coverage incomplete"):
            capture.check_schema_report(report, paths)


class RecordedVerificationTests(unittest.TestCase):
    def setUp(self):
        self.path = (native.ROOT / "target/offline-session").resolve()
        self.args = SimpleNamespace(min_chunks=1, schema_timeout=240, max_log_mib=1)
        self.coverage = dict.fromkeys(("status_response", "status_pong", "login_finished", "login_acknowledged",
                                      "network_registry_load", "configuration_acknowledged", "play_login"), True)
        self.coverage["native_terrain_packets"] = 1
        self.events = [{"sequence": 0, "state": "play", "direction": "clientbound", "packet_id": 42},
                       {"sequence": 1, "state": "play", "direction": "serverbound", "packet_id": 10}]
        self.wire = json.dumps({"events": self.events}).encode("utf-8")
        packets = [{**self.events[0], "status": "pass", "packet": "minecraft:level_chunk_with_light"},
                   {**self.events[1], "status": "pass", "packet": "minecraft:chunk_batch_received"}]
        self.row = {"status": "pass", "decoded": 2, "expected": 2, "capture_path": str(self.path),
                    "coverage": self.coverage, "packets": packets, "wire_index_sha256": capture.digest(self.wire)}

    def verify(self):
        report = json.dumps({"minecraft": "26.2", "protocol": 776, "sessions": [self.row]}).encode("utf-8")
        with patch.object(capture, "execute_owned"), patch.object(Path, "read_bytes", side_effect=(report, self.wire)):
            return capture.verify_sessions(native.ROOT / "target/new-output", {}, self.args, ["ProtocolSchema"], [self.path])

    def test_complete_recorded_schema_has_exact_wire_hash_count_and_chunk_coverage(self):
        rows = self.verify()
        self.assertEqual(rows[0]["coverage"]["native_terrain_packets"], 1)

    def test_valid_frame_summary_cannot_hide_changed_wire_index(self):
        self.row["wire_index_sha256"] = "different"
        with self.assertRaisesRegex(wire.ProtocolError, "wire index/count differs"):
            self.verify()

    def test_valid_frame_summary_cannot_skip_or_reorder_original_events(self):
        self.row["packets"] = list(reversed(self.row["packets"]))
        with self.assertRaisesRegex(wire.ProtocolError, "packet sequence differs"):
            self.verify()

    def test_all_recorded_chunks_must_have_native_terrain_validation(self):
        self.coverage["native_terrain_packets"] = 2
        with self.assertRaisesRegex(wire.ProtocolError, "terrain coverage differs"):
            self.verify()

    def test_native_failure_prevents_reverify_pass_and_is_preserved(self):
        output = native.ROOT / "target/offline-reverify"
        inputs = {"path": str(self.path), "original_status": "fail", "files": {"results.json": {"sha256": "old"}}}
        args = SimpleNamespace(capture=self.path)
        with patch.object(capture, "recorded_inputs", return_value=([self.path], inputs)), \
                patch.object(capture, "HELPERS", ()), patch.object(Path, "mkdir"), patch.object(Path, "rglob", return_value=[]), \
                patch.object(capture, "identity", return_value={}), patch.object(capture, "isolated_environment", return_value={}), \
                patch.object(capture, "prepare_schema", return_value=(None, [])), \
                patch.object(capture, "verify_sessions", side_effect=wire.ProtocolError("Missing builtin tag")), \
                patch.object(capture, "verify_source_freeze", return_value={"status": "pass"}), \
                patch.object(native, "sha", return_value="old"), patch.object(native, "write_json") as write, \
                patch.object(capture, "write_bytes"), patch.object(capture, "start_server", side_effect=AssertionError("server launched")):
            result = capture.reverify_run(output, args, {}, [])
        self.assertEqual(result["status"], "fail")
        self.assertEqual(result["native_session_validation"], "fail")
        self.assertEqual(result["original_capture_status"], "fail")
        self.assertIn("Missing builtin tag", result["error"])
        self.assertTrue(all(operation.args[0].is_relative_to(output) for operation in write.call_args_list))


class RecordedInputTests(unittest.TestCase):
    def setUp(self):
        self.root = (native.ROOT / "target/offline-capture").resolve()
        self.session = self.root / "session-1"
        self.files = {"inputs.json", "helpers/native.py", "session-1/wire.json", "session-1/play.clientbound.frames",
                      *("session-1/" + name for name in ("auth.json", "config.json", "play.json", "outcome.json",
                                                       "status.json", "socket-outcomes.json"))}
        self.receipts = {name: {"bytes": 1, "sha256": "old"} for name in self.files}
        self.inputs = {"minecraft": {"name": "26.2", "protocol_version": 776}, "jar": {"sha256": native.JAR_SHA256},
                       "library_count": 39, "libraries": [{"sha256": "library"}] * 39,
                       "helpers": {"native.py": {"sha256": "old"}}}
        self.documents = {self.root / "inputs.json": self.inputs, self.root / "manifest.json": {"files": self.receipts},
                          self.session / "wire.json": {"events": [{"file": "play.clientbound.frames"}]},
                          self.root / "results.json": {"status": "fail", "error": "native registry load failed"}}
        self.read_patch = patch.object(Path, "read_bytes", autospec=True,
                                      side_effect=lambda path: json.dumps(self.documents[path]).encode("utf-8"))
        self.read_patch.start()
        self.addCleanup(self.read_patch.stop)
        self.patches = [patch.object(Path, "glob", return_value=[self.session]),
                        patch.object(Path, "is_dir", return_value=True), patch.object(Path, "is_file", return_value=True),
                        patch.object(Path, "is_symlink", return_value=False),
                        patch.object(native, "sha", return_value="library")]
        for patched in self.patches:
            patched.start()
            self.addCleanup(patched.stop)

    def identity(self, path):
        name = path.relative_to(self.root).as_posix()
        return {"path": str(path), **self.receipts.get(name, {"bytes": 1, "sha256": "root-receipt"})}

    def test_original_helper_hashes_are_checked_without_requiring_current_native_source_match(self):
        with patch.object(capture, "identity", side_effect=self.identity):
            sessions, receipt = capture.recorded_inputs(self.root, [Path("library.jar")] * 39)
        self.assertEqual(sessions, [self.session])
        self.assertEqual(receipt["original_status"], "fail")
        self.assertEqual(receipt["files"]["helpers/native.py"]["sha256"], "old")
        self.assertIn("results.json", receipt["files"])
        self.assertIn("manifest.json", receipt["files"])

    def test_changed_recorded_frame_is_rejected_before_native_execution(self):
        def changed(path):
            result = self.identity(path)
            if path.suffix == ".frames":
                result["sha256"] = "changed"
            return result

        with patch.object(capture, "identity", side_effect=changed), self.assertRaisesRegex(wire.ProtocolError, "receipt differs"):
            capture.recorded_inputs(self.root, [Path("library.jar")] * 39)

    def test_original_helper_and_manifest_hashes_must_agree(self):
        self.inputs["helpers"]["native.py"]["sha256"] = "different"
        with patch.object(capture, "identity", side_effect=self.identity), self.assertRaisesRegex(wire.ProtocolError, "helper receipt differs"):
            capture.recorded_inputs(self.root, [Path("library.jar")] * 39)


class PersistenceWatchTests(unittest.TestCase):
    def test_watched_local_palette_uses_y_z_x_order_and_preserves_counts(self):
        x, y, z = 5, 2, 4
        wanted = y * 256 + z * 16 + x
        packed = [0] * 256
        packed[wanted // 16] = 1 << ((wanted % 16) * 4)
        section = (struct.pack(">hh", 4096, 0) + b"\x04" + wire.varint(2) + wire.varint(1) + wire.varint(7)
                   + struct.pack(">256Q", *packed) + b"\x00" + wire.varint(40))
        raw = struct.pack(">ii", 0, 0) + b"\x00" + wire.varint(len(section)) + section + bytes(7)
        result = wire.chunk(raw, height=16, block_states=32366, biomes=66, watch=(x, y, z))
        self.assertEqual(result["watched_block_state"], 7)
        self.assertEqual(result["sections"][0]["block_states"], {1: 4095, 7: 1})
        other = wire.chunk(raw, height=16, block_states=32366, biomes=66, watch=(x, y, z + 1))
        self.assertEqual(other["watched_block_state"], 1)

    def test_watched_singleton_palette_and_invalid_coordinate_admission(self):
        self.assertEqual(wire.chunk(flat_chunk(), height=384, block_states=32366, biomes=66,
                                   watch=(0, 0, 0))["watched_block_state"], 1)
        self.assertEqual(wire.chunk(flat_chunk(), height=384, block_states=32366, biomes=66,
                                   watch=(15, 383, 15))["watched_block_state"], 0)
        for position in ((-1, 0, 0), (16, 0, 0), (0, 384, 0), (0, -1, 0), (True, 0, 0)):
            with self.subTest(position=position), self.assertRaises(wire.ProtocolError):
                wire.chunk(flat_chunk(), height=384, block_states=32366, biomes=66, watch=position)

    def test_mutation_cannot_claim_a_different_chunk_or_unchanged_save(self):
        output = (native.ROOT / "target/offline-persistence").resolve()
        args = SimpleNamespace(seed=17, persistence_probe=Path("probe.exe"), max_log_mib=1)
        play = {"state_packets": {"player_position": {"x": 10.5, "z": -3.5}}}
        mutation = {"schema": "bcore.persistence-mutation.v1", "format_version": 6, "minecraft": "26.2",
                    "protocol": 776, "data_version": 4903, "seed": "17", "chunk": [0, -1],
                    "path": str(output / "world/chunks/c.0.-1.bcc"), "local": [7, 0, 7], "block": [7, 0, -9],
                    "before_state": 1, "state": 2, "before_sha256": "before", "sha256": "after"}
        for change in ({}, {"chunk": [1, -1]}, {"sha256": "before"}, {"state": 1}):
            with self.subTest(change=change), patch.object(capture, "execute_owned"), \
                    patch.object(Path, "read_bytes", side_effect=(json.dumps(play).encode(), json.dumps({**mutation, **change}).encode())), \
                    patch.object(Path, "samefile", return_value=True), patch.object(native, "sha", return_value="after"), \
                    patch.object(native, "write_json") as write:
                if change:
                    with self.assertRaises(wire.ProtocolError):
                        capture.mutate_saved_chunk(output, {}, args)
                    write.assert_not_called()
                else:
                    observed = capture.mutate_saved_chunk(output, {}, args)
                    self.assertEqual(observed["reported_path"], mutation["path"])
                    self.assertEqual(observed["state"], mutation["state"])

    def test_windows_verbatim_path_alias_requires_same_file_identity(self):
        output = (native.ROOT / "target/offline-verbatim").resolve()
        expected = output / "world/chunks/c.0.-1.bcc"
        args = SimpleNamespace(seed=17, persistence_probe=Path("probe.exe"), max_log_mib=1)
        play = {"state_packets": {"player_position": {"x": 10.5, "z": -3.5}}}
        mutation = {"schema": "bcore.persistence-mutation.v1", "format_version": 6, "minecraft": "26.2",
                    "protocol": 776, "data_version": 4903, "seed": "17", "chunk": [0, -1],
                    "path": "\\\\?\\" + str(expected), "local": [7, 0, 7], "block": [7, 0, -9],
                    "before_state": 1, "state": 2, "before_sha256": "before", "sha256": "after"}
        for same in (True, False):
            with self.subTest(same=same), patch.object(capture, "execute_owned"), \
                    patch.object(Path, "read_bytes", side_effect=(json.dumps(play).encode(), json.dumps(mutation).encode())), \
                    patch.object(Path, "samefile", return_value=same), patch.object(native, "sha", return_value="after"), \
                    patch.object(native, "write_json"):
                if same:
                    result = capture.mutate_saved_chunk(output, {}, args)
                    self.assertEqual(result["path"], str(expected))
                    self.assertEqual(result["reported_path"], mutation["path"])
                else:
                    with self.assertRaises(wire.ProtocolError):
                        capture.mutate_saved_chunk(output, {}, args)


class SourceFreezeTests(unittest.TestCase):
    def test_executable_freeze_rejects_changed_or_missing_binary(self):
        path = Path("frozen-bcore.exe")
        expected = {"bytes": 10, "sha256": "same"}
        with patch.object(capture, "identity", return_value={"bytes": 10, "sha256": "same"}):
            self.assertEqual(capture.executable_freeze(path, expected)["status"], "pass")
        with patch.object(capture, "identity", return_value={"bytes": 10, "sha256": "different"}):
            self.assertEqual(capture.executable_freeze(path, expected)["status"], "fail")
        with patch.object(capture, "identity", side_effect=OSError("missing binary")):
            result = capture.executable_freeze(path, expected)
        self.assertEqual(result["status"], "fail")
        self.assertIn("missing binary", result["error"])

    def test_completion_hashes_report_changed_and_missing_sources(self):
        sources = (Path("protocol_capture.py"), Path("ProtocolSchema.java"), Path("test_protocol_capture.py"))
        expected = {path.name: {"sha256": str(index)} for index, path in enumerate(sources)}
        with patch.object(capture, "HELPERS", sources), patch.object(native, "sha", side_effect=("0", "changed", OSError("missing"))), \
                patch.object(native, "write_json") as write:
            result = capture.verify_source_freeze(Path("memory-output"), expected)
        self.assertEqual(result["status"], "fail")
        self.assertEqual(result["changed"], ["ProtocolSchema.java", "test_protocol_capture.py"])
        self.assertIsNone(result["observed_sha256"]["test_protocol_capture.py"])
        self.assertEqual(result["expected_sha256"]["ProtocolSchema.java"], "1")
        write.assert_called_once_with(Path("memory-output/source-freeze.json"), result)

    def test_unchanged_completion_hashes_match_frozen_receipt(self):
        source = Path("protocol_capture.py")
        with patch.object(capture, "HELPERS", (source,)), patch.object(native, "sha", return_value="same"), \
                patch.object(native, "write_json"):
            result = capture.verify_source_freeze(Path("memory-output"), {source.name: {"sha256": "same"}})
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["changed"], [])


if __name__ == "__main__":
    unittest.main()
