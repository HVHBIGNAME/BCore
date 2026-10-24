"""Fresh 26.2 packet fixtures and a bounded BCore schema/client-smoke probe.

Launch only through scripts/run_generation_check.py. A queue owner selects the
command; this driver does not enqueue itself. The socket probe and native schema
decoder are not a Minecraft GUI client.
"""
from __future__ import annotations

import argparse
from collections import Counter
from contextlib import ExitStack
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import socket
import struct
import sys
import time
import traceback
import uuid

import native
from protocol_runtime import OwnedProcess, require_listener, reserve_port, settle_listener, wait_ready
import protocol_wire as wire

DEFAULT_SEED = 846692123413862008
LIBRARY_COUNT = 39
HERE = Path(__file__).resolve().parent
HELPERS = (Path(__file__).resolve(), HERE / "protocol_runtime.py", HERE / "protocol_wire.py",
           HERE / "ProtocolSchema.java", HERE / "test_protocol_capture.py", Path(native.__file__).resolve())
REQUIRED_STATE = frozenset(("login", "player_position", "player_info_update", "commands",
                            "set_default_spawn_position", "set_chunk_cache_center", "player_abilities",
                            "set_health", "set_time"))


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def identity(path: Path) -> dict:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": native.sha(path)}


def write_bytes(path: Path, value: bytes) -> None:
    with path.open("xb") as stream:
        stream.write(value)


def json_object(raw: bytes) -> dict:
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise wire.ProtocolError(f"duplicate JSON key {key!r}")
            result[key] = value
        return result
    def reject_constant(value):
        raise wire.ProtocolError(f"non-finite JSON {value}")
    value = json.loads(raw.decode("utf-8", "strict"), object_pairs_hook=unique, parse_constant=reject_constant)
    if not isinstance(value, dict):
        raise wire.ProtocolError("JSON object required")
    return value


class Capture:
    def __init__(self, output: Path, packet_limit: int, byte_limit: int):
        output.mkdir()
        self.output, self.packet_limit, self.byte_limit = output, packet_limit, byte_limit
        self.stack = ExitStack()
        self.raw = {}
        self.binary = {}
        self.counts = Counter()
        self.events = []
        self.total_bytes = 0
        self.sockets = []
        self.started = time.monotonic()
        for state in ("handshake", "status", "login", "configuration", "play"):
            for direction in ("clientbound", "serverbound"):
                name = f"{state}.{direction}.frames"
                self.raw[state, direction] = self.stack.enter_context((output / name).open("xb"))
                if direction == "clientbound" and state in ("login", "configuration", "play"):
                    filename = "config_packets.bin" if state == "configuration" else f"{state}_packets.bin"
                    stream = self.stack.enter_context((output / filename).open("xb"))
                    stream.write(struct.pack(">I", 0))
                    self.binary[state] = stream

    def record(self, state: str, direction: str, packet_id: int, payload: bytes, framed: bytes) -> dict:
        if len(self.events) >= self.packet_limit or self.total_bytes + len(framed) > self.byte_limit:
            raise wire.ProtocolError("capture packet/byte budget exceeded")
        stream = self.raw[state, direction]
        offset = stream.tell()
        stream.write(framed)
        stream.flush()
        self.total_bytes += len(framed)
        event = {"sequence": len(self.events), "elapsed_ms": round((time.monotonic() - self.started) * 1000, 3),
                 "state": state, "direction": direction, "packet_id": packet_id,
                 "file": Path(stream.name).name, "offset": offset, "frame_length": len(framed),
                 "frame_sha256": digest(framed), "payload_length": len(payload), "payload_sha256": digest(payload)}
        self.events.append(event)
        if direction == "clientbound" and state in self.binary and not (
                state == "login" and packet_id == 0):
            self.binary[state].write(struct.pack(">iI", packet_id, len(payload)) + payload)
            self.binary[state].flush()
            self.counts[state] += 1
        return event

    def close(self, outcome: dict) -> None:
        for state, stream in self.binary.items():
            stream.seek(0)
            stream.write(struct.pack(">I", self.counts[state]))
        self.stack.close()
        native.write_json(self.output / "wire.json", {"compression": "disabled", "events": self.events})
        native.write_json(self.output / "socket-outcomes.json", self.sockets)
        native.write_json(self.output / "outcome.json", outcome)


class Connection:
    def __init__(self, owner: OwnedProcess, port: int, capture: Capture, timeout: float):
        self.owner, self.port, self.capture, self.timeout = owner, port, capture, timeout
        self.state = "handshake"
        self.buffer = bytearray()
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.socket.settimeout(timeout)
        self.socket.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.outcome = {"connect_started_unix": time.time(), "host": "127.0.0.1", "port": port,
                        "end": "not_connected", "partial_frame_hex": "",
                        "connection": len(capture.sockets), "received_bytes": 0}
        capture.sockets.append(self.outcome)
        try:
            self.outcome["ownership_before"] = require_listener(owner.process.pid, port)
            self.socket.connect(("127.0.0.1", port))
            self.outcome.update(local=list(self.socket.getsockname()), peer=list(self.socket.getpeername()),
                                connected_unix=time.time(), end="connected")
            self.outcome["ownership_after"] = require_listener(owner.process.pid, port)
        except BaseException as error:
            self.outcome.update(end="connect_failed", error=f"{type(error).__name__}: {error}")
            self.socket.close()
            raise

    def send(self, packet_id: int, payload: bytes = b"") -> dict:
        framed = wire.frame(packet_id, payload)
        # Reserve capture admission before putting bytes on the wire.
        if len(self.capture.events) >= self.capture.packet_limit or self.capture.total_bytes + len(framed) > self.capture.byte_limit:
            raise wire.ProtocolError("capture packet/byte budget exceeded before send")
        self.socket.sendall(framed)
        event = self.capture.record(self.state, "serverbound", packet_id, payload, framed)
        event["connection"] = self.outcome["connection"]
        return event

    def receive(self, deadline: float) -> tuple[int, bytes, dict]:
        while True:
            framed = wire.split_frame(self.buffer)
            if framed is not None:
                size, packet_id, payload = framed
                raw = bytes(self.buffer[:size])
                del self.buffer[:size]
                event = self.capture.record(self.state, "clientbound", packet_id, payload, raw)
                event["connection"] = self.outcome["connection"]
                return packet_id, payload, event
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self.outcome["last_timeout"] = {"state": self.state, "partial_bytes": len(self.buffer), "unix": time.time()}
                raise TimeoutError(f"{self.state} client deadline exceeded with {len(self.buffer)} partial bytes")
            self.owner.check()
            self.socket.settimeout(min(self.timeout, remaining, 0.25))
            try:
                prefix = wire.frame_prefix(self.buffer)
                wanted = 1 if prefix is None else min(65536, prefix[0] - len(self.buffer))
                block = self.socket.recv(wanted)
            except socket.timeout:
                continue
            if not block:
                self.outcome.update(end="peer_eof", eof_state=self.state, eof_unix=time.time())
                if self.buffer:
                    raise wire.ProtocolError(f"EOF inside {self.state} frame ({len(self.buffer)} bytes)")
                raise EOFError(f"peer closed during {self.state}")
            self.buffer.extend(block)
            self.outcome["received_bytes"] += len(block)
            if len(self.buffer) > wire.MAX_FRAME + 3:
                raise wire.ProtocolError("receive buffer budget exceeded")

    def handshake(self, state: str, protocol: int = 776) -> dict:
        next_state = 1 if state == "status" else 2
        event = self.send(0, wire.varint(protocol) + wire.string("127.0.0.1", 255)
                          + struct.pack(">H", self.port) + wire.varint(next_state))
        self.state = state
        return event

    def close(self) -> None:
        self.outcome["partial_frame_hex"] = bytes(self.buffer).hex()
        if self.outcome["end"] == "connected":
            self.outcome["end"] = "client_close"
        self.socket.close()
        self.outcome["closed_unix"] = time.time()


class PacketInventory:
    def __init__(self, data: dict):
        if (data.get("minecraft"), data.get("protocol")) != ("26.2", 776):
            raise wire.ProtocolError("native packet inventory identity differs")
        self.data = data
        self.names = {}
        self.ids = {}
        for state, directions in data["packets"].items():
            for direction, packets in directions.items():
                for name, definition in packets.items():
                    packet_id = definition["protocol_id"]
                    if (state, direction, packet_id) in self.names:
                        raise wire.ProtocolError("duplicate native packet ID")
                    self.names[state, direction, packet_id] = name.removeprefix("minecraft:")
                    self.ids[state, direction, name.removeprefix("minecraft:")] = packet_id
        registries = data["synchronized_registries"]
        if not registries or len(set(registries)) != len(registries):
            raise wire.ProtocolError("empty/duplicate synchronized registry inventory")

    def name(self, state: str, packet_id: int) -> str:
        try:
            return self.names[state, "clientbound", packet_id]
        except KeyError as error:
            raise wire.ProtocolError(f"unknown native {state} packet 0x{packet_id:02x}") from error

    def send(self, connection: Connection, name: str, payload: bytes = b"") -> dict:
        return connection.send(self.ids[connection.state, "serverbound", name], payload)


def settings(radius: int) -> bytes:
    return wire.string("en_us", 16) + struct.pack(">b", radius) + wire.varint(0) + bytes((1, 127, 1, 0, 1, 0))


def hashed_seed(seed: int) -> int:
    return struct.unpack("<q", hashlib.sha256(struct.pack("<q", seed)).digest()[:8])[0]


class Session:
    def __init__(self, owner: OwnedProcess, port: int, inventory: PacketInventory, args, output: Path,
                 expected_block: dict | None = None):
        self.owner, self.port, self.inventory, self.args = owner, port, inventory, args
        self.capture = Capture(output, args.max_packets, args.max_capture_mib * 1024 * 1024)
        self.cookies = {}
        self.auth = {"status": "not_attempted", "actual_full_client": False, "login_schema": "26.2 native profile + sessionUUID"}
        self.config = {"status": "not_attempted", "registries": [], "known_packs_response": []}
        self.play = {"status": "not_attempted", "state_packets": {}, "batches": [], "chunks": [], "checks": [],
                     "actual_full_client": False, "full_schema": "pending_native_decode"}
        self.socket = None
        self.seen = set()
        self.batch = None
        self.teleport = None
        self.loaded = False
        self.bundle_open = False
        self.connections = []
        self.geometry_cache = None
        self.expected_block = expected_block
        self.persisted_block_observed = False
        self.phase = "not_started"
        self.status_probe = {"status": "not_attempted", "attempts": [], "actual_full_client": False,
                             "max_attempts": 2 if args.mode == "official" else 1,
                             "retry_settle_seconds": args.status_retry_settle}
        self.gates = Capture(output / "version-gates", args.max_packets, args.max_capture_mib * 1024 * 1024) \
            if args.mode == "bcore" else None
        self.player_list = {}
        self.cached_chunks = set()

    def connect(self) -> Connection:
        connection = Connection(self.owner, self.port, self.capture, self.args.socket_timeout)
        self.connections.append(connection)
        return connection

    def common(self, name: str, payload: bytes) -> bool:
        connection = self.socket
        if name == "cookie_request":
            key = wire.cookie_key(payload)
            self.inventory.send(connection, "cookie_response", wire.cookie_response(key, self.cookies.get(key)))
        elif name == "store_cookie":
            key, value = wire.store_cookie(payload)
            self.cookies[key] = value
        elif name in ("keep_alive", "ping"):
            reader = wire.Reader(payload)
            value = reader.number("q" if name == "keep_alive" else "i")
            reader.finish()
            self.inventory.send(connection, "keep_alive" if name == "keep_alive" else "pong", payload)
            self.play["checks"].append({"name": f"{connection.state}_{name}_reply", "value": str(value)})
        elif name in ("transfer", "resource_pack_push"):
            raise wire.ProtocolError(f"unexpected {connection.state}/{name} in isolated no-pack fixture")
        elif name in ("disconnect", "login_disconnect"):
            self.capture.sockets[-1]["disconnect"] = {"state": connection.state, "payload_hex": payload.hex()}
            raise wire.ProtocolError(f"server sent {connection.state}/{name}: {payload.hex()}")
        else:
            return False
        return True

    def status_attempt(self, attempt: dict) -> dict:
        connection = None
        attempt.update(status="running", phase="connect", started_unix=time.time())
        try:
            connection = self.connect()
            attempt["connection"] = connection.outcome["connection"]
            connection.outcome["purpose"] = "status"
            connection.outcome["status_attempt"] = attempt["attempt"]
            handshake = connection.handshake("status")
            request = connection.send(0)
            attempt.update(phase="response", handshake_sequence=handshake["sequence"],
                           handshake_frame_sha256=handshake["frame_sha256"], request_sequence=request["sequence"],
                           request_frame_sha256=request["frame_sha256"])
            deadline = time.monotonic() + self.args.socket_timeout
            packet_id, payload, event = connection.receive(deadline)
            attempt.update(phase="response_validation", response_sequence=event["sequence"])
            if packet_id != 0:
                raise wire.ProtocolError(f"status response ID differs: {packet_id}")
            reader = wire.Reader(payload)
            result = json_object(reader.string().encode("utf-8"))
            reader.finish()
            version = result.get("version", {})
            if (version.get("name"), version.get("protocol")) != ("26.2", 776):
                raise wire.ProtocolError(f"status version gate failed: {version}")
            ping = struct.pack(">q", time.time_ns() // 1_000_000)
            sent = connection.send(1, ping)
            attempt.update(phase="ping", ping_sequence=sent["sequence"])
            packet_id, payload, event = connection.receive(deadline)
            if packet_id != 1 or payload != ping:
                raise wire.ProtocolError("status ping echo differs")
            attempt.update(status="pass", phase="complete", pong_sequence=event["sequence"])
            return result
        except BaseException as error:
            attempt.update(status="fail", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            if connection is not None:
                connection.close()
                attempt.update(partial_frame_hex=connection.outcome["partial_frame_hex"],
                               received_bytes=connection.outcome["received_bytes"], socket_end=connection.outcome["end"])
            attempt["finished_unix"] = time.time()

    def status(self) -> dict:
        self.phase = "status"
        probe = self.status_probe
        probe["status"] = "running"
        for number in range(1, probe["max_attempts"] + 1):
            attempt = {"attempt": number}
            probe["attempts"].append(attempt)
            try:
                result = self.status_attempt(attempt)
            except (EOFError, TimeoutError):
                probe.update(status="fail", initial_status=probe["attempts"][0]["status"])
                if (number == probe["max_attempts"] or attempt["phase"] != "response"
                        or attempt["received_bytes"] or attempt["partial_frame_hex"]):
                    raise
                try:
                    attempt["retry_settle"] = settle_listener(self.owner, self.port, probe["retry_settle_seconds"])
                except BaseException as error:
                    probe["retry_error"] = f"{type(error).__name__}: {error}"
                    raise
            except BaseException:
                probe.update(status="fail", initial_status=probe["attempts"][0]["status"])
                raise
            else:
                probe.update(status="pass" if number == 1 else "pass_after_retry",
                             initial_status=probe["attempts"][0]["status"], successful_attempt=number)
                return result
        raise RuntimeError("status attempt budget exhausted")

    def version_gate(self, protocol: int) -> dict:
        if self.gates is None:
            raise RuntimeError("version gate requires BCore mode")
        connection = Connection(self.owner, self.port, self.gates, self.args.socket_timeout)
        start_sequence = len(self.gates.events)
        try:
            connection.handshake("login", protocol)
            connection.send(0, wire.string(self.args.username, 16) + self.args.player_uuid.bytes)
            packet_id, payload, _ = connection.receive(time.monotonic() + self.args.socket_timeout)
            if packet_id != self.inventory.ids["login", "clientbound", "login_disconnect"]:
                raise wire.ProtocolError(f"protocol {protocol} was not rejected in login: {packet_id}")
            reader = wire.Reader(payload)
            reason = json_object(reader.string().encode("utf-8"))
            reader.finish()
            for event in self.gates.events[start_sequence:]:
                event["purpose"] = "expected_version_gate_rejection"
            return {"protocol": protocol, "result": "rejected", "reason": reason}
        finally:
            connection.close()

    def login(self) -> None:
        self.phase = "login"
        self.auth["status"] = "running"
        self.socket = self.connect()
        self.socket.handshake("login")
        self.inventory.send(self.socket, "hello", wire.string(self.args.username, 16) + self.args.player_uuid.bytes)
        deadline = time.monotonic() + self.args.client_timeout
        while True:
            packet_id, payload, _ = self.socket.receive(deadline)
            name = self.inventory.name("login", packet_id)
            if self.common(name, payload):
                continue
            if name == "login_compression":
                raise wire.ProtocolError("compression enabled despite network-compression-threshold=-1")
            if name == "hello":
                raise wire.ProtocolError("server requested encryption in offline-mode fixture")
            if name == "custom_query":
                reader = wire.Reader(payload)
                query = reader.count(0x7fffffff)
                reader.identifier()
                reader.take(reader.remaining)
                self.inventory.send(self.socket, "custom_query_answer", wire.varint(query) + b"\x00")
                continue
            if name != "login_finished":
                raise wire.ProtocolError(f"unexpected login packet {name}")
            self.auth.update(wire.login_finished(payload),
                             requested_uuid=str(self.args.player_uuid), requested_username=self.args.username)
            if self.auth["username"] != self.args.username:
                raise wire.ProtocolError("login profile name differs from request")
            self.inventory.send(self.socket, "login_acknowledged")
            self.socket.state = "configuration"
            self.auth["status"] = "pass"
            return

    def configuration(self) -> None:
        self.phase = "configuration"
        self.config["status"] = "running"
        self.inventory.send(self.socket, "client_information", settings(self.args.radius))
        deadline = time.monotonic() + self.args.client_timeout
        known, registries = False, set()
        while True:
            packet_id, payload, event = self.socket.receive(deadline)
            name = self.inventory.name("configuration", packet_id)
            if self.common(name, payload):
                continue
            if name == "select_known_packs":
                if known or registries:
                    raise wire.ProtocolError("known-packs selection repeated or after registry data")
                self.config["offered_known_packs"] = wire.known_packs(payload)
                self.inventory.send(self.socket, "select_known_packs", wire.varint(0))
                known = True
            elif name == "registry_data":
                if not known:
                    raise wire.ProtocolError("registry before empty known-packs response")
                decoded = wire.registry_data(payload)
                if decoded["registry"] in registries:
                    raise wire.ProtocolError("repeated configuration registry")
                registries.add(decoded["registry"])
                decoded["sequence"] = event["sequence"]
                self.config["registries"].append(decoded)
            elif name == "update_enabled_features":
                self.config["features"] = wire.enabled_features(payload)
            elif name == "update_tags":
                self.config["tags"] = wire.tags(payload)
            elif name == "code_of_conduct":
                reader = wire.Reader(payload)
                self.config["code_of_conduct"] = reader.string()
                reader.finish()
                self.inventory.send(self.socket, "accept_code_of_conduct")
            elif name == "finish_configuration":
                if payload:
                    raise wire.ProtocolError("finish-configuration must be empty")
                expected = set(self.inventory.data["synchronized_registries"])
                if registries != expected:
                    raise wire.ProtocolError(f"synchronized registries differ: missing {sorted(expected - registries)}, extra {sorted(registries - expected)}")
                if "features" not in self.config or "tags" not in self.config:
                    raise wire.ProtocolError("configuration omitted enabled features/tags")
                self.config.update(status="pass", registry_count=len(registries), all_registry_entries_have_data=True)
                self.inventory.send(self.socket, "finish_configuration")
                self.socket.state = "play"
                return

    def geometry(self) -> int:
        if self.geometry_cache is not None:
            return self.geometry_cache
        login = self.play["state_packets"]["login"]
        registry = next(row for row in self.config["registries"] if row["registry"] == "minecraft:dimension_type")
        index = login["dimension_type"]
        if index >= len(registry["entries"]):
            raise wire.ProtocolError("play dimension ID outside configuration registry")
        definition = registry["entries"][index]["data"]["Compound"]
        height, min_y = definition["height"]["Int"], definition["min_y"]["Int"]
        if not 16 <= height <= 4096 or height % 16 or min_y % 16:
            raise wire.ProtocolError("invalid dimension geometry")
        self.play["dimension_geometry"] = {"height": height, "min_y": min_y}
        self.geometry_cache = height
        return height

    def receive_play(self, deadline: float) -> tuple[str, dict, dict]:
        packet_id, payload, event = self.socket.receive(deadline)
        name = self.inventory.name("play", packet_id)
        self.seen.add(name)
        decoded = {}
        if self.common(name, payload):
            return name, decoded, event
        if name == "bundle_delimiter":
            if payload:
                raise wire.ProtocolError("bundle delimiter must be empty")
            self.bundle_open = not self.bundle_open
        elif name == "login":
            if "login" in self.play["state_packets"]:
                raise wire.ProtocolError("repeated play login")
            decoded = wire.play_login(payload)
            if decoded["onlineMode"] or decoded["enforcesSecureChat"]:
                raise wire.ProtocolError("play login requires online/secure chat in offline fixture")
            if decoded["hashed_seed"] != hashed_seed(self.args.seed):
                raise wire.ProtocolError("play login hashed seed differs from selected seed")
        elif name == "player_position":
            decoded = wire.position(payload)
            if decoded["flags"] & 7:
                if self.teleport is None:
                    raise wire.ProtocolError("relative initial teleport has no base")
                for index, axis in enumerate(("x", "y", "z")):
                    if decoded["flags"] & (1 << index):
                        decoded[axis] += self.teleport[axis]
            self.teleport = decoded
            self.inventory.send(self.socket, "accept_teleportation", bytes.fromhex(decoded["confirm_hex"]))
        elif name == "chunk_batch_start":
            if payload or self.batch is not None:
                raise wire.ProtocolError("invalid/nested chunk-batch start")
            self.batch = []
        elif name == "level_chunk_with_light":
            if self.batch is None or "login" not in self.play["state_packets"]:
                raise wire.ProtocolError("chunk outside a batch or before play login")
            biome_registry = next(row for row in self.config["registries"] if row["registry"] == "minecraft:worldgen/biome")
            height = self.geometry()
            expected = self.expected_block
            watch = None
            if expected is not None and len(payload) >= 8 and list(struct.unpack(">ii", payload[:8])) == expected["chunk"]:
                local = expected["local"]
                watch = (local[0], local[1] - self.play["dimension_geometry"]["min_y"], local[2])
            decoded = wire.chunk(payload, height=height, block_states=32366,
                                 biomes=len(biome_registry["entries"]), watch=watch)
            if watch is not None:
                if decoded["watched_block_state"] != expected["state"]:
                    raise wire.ProtocolError(f"persisted block differs after restart: {decoded['watched_block_state']} != {expected['state']}")
                self.persisted_block_observed = True
                self.play["checks"].append({"name": "persisted block after restart", "status": "pass",
                                            "sequence": event["sequence"], "block": expected["block"], "state": expected["state"]})
            decoded["sequence"] = event["sequence"]
            self.batch.append([decoded["x"], decoded["z"]])
            self.play["chunks"].append(decoded)
            self.cached_chunks.add((decoded["x"], decoded["z"]))
        elif name == "chunk_batch_finished":
            reader = wire.Reader(payload)
            count = reader.count(self.args.max_chunks)
            reader.finish()
            if self.batch is None or count != len(self.batch) or count == 0:
                raise wire.ProtocolError(f"completed batch count {count} differs from received chunks {self.batch}")
            self.play["batches"].append({"chunks": self.batch, "count": count, "sequence": event["sequence"]})
            self.batch = None
            self.inventory.send(self.socket, "chunk_batch_received", struct.pack(">f", self.args.chunks_per_tick))
        elif name == "player_info_update":
            decoded = wire.player_info(payload)
            for row in decoded["entries"]:
                self.player_list.setdefault(row["uuid"], {}).update(row)
        elif name == "player_info_remove":
            reader = wire.Reader(payload)
            removed = [reader.uuid() for _ in range(reader.count(1024))]
            reader.finish()
            for key in removed:
                self.player_list.pop(key, None)
            decoded = {"removed": removed}
        elif name == "set_time":
            decoded = wire.world_time(payload)
        elif name in ("system_chat", "player_chat", "disguised_chat"):
            decoded = wire.chat(payload, name)
        elif name == "set_default_spawn_position":
            reader = wire.Reader(payload)
            decoded = wire.global_position(reader)
            decoded.update(yaw=reader.number("f"), pitch=reader.number("f"))
            reader.finish()
        elif name == "set_chunk_cache_center":
            reader = wire.Reader(payload)
            decoded = {"x": reader.varint(), "z": reader.varint()}
            reader.finish()
        elif name == "forget_level_chunk":
            reader = wire.Reader(payload)
            z, x = reader.number("i"), reader.number("i")
            reader.finish()
            self.cached_chunks.discard((x, z))
            decoded = {"x": x, "z": z}
        elif name == "player_abilities":
            reader = wire.Reader(payload)
            decoded = {"flags": reader.byte(), "flying_speed": reader.number("f"), "walking_speed": reader.number("f")}
            reader.finish()
        elif name == "set_health":
            reader = wire.Reader(payload)
            decoded = {"health": reader.number("f"), "food": reader.count(20), "saturation": reader.number("f")}
            reader.finish()
        elif name == "game_event":
            reader = wire.Reader(payload)
            decoded = {"event": reader.byte(), "value": reader.number("f")}
            reader.finish()
        elif name == "pong_response":
            reader = wire.Reader(payload)
            decoded = {"id": reader.number("q")}
            reader.finish()
        elif name == "start_configuration":
            raise wire.ProtocolError("unexpected reconfiguration in bounded initial probe")
        if name in REQUIRED_STATE:
            self.play["state_packets"][name] = decoded or {"sequence": event["sequence"], "native_decode": "pending"}
        if self.teleport is not None and self.play["batches"] and not self.loaded:
            self.inventory.send(self.socket, "player_loaded")
            self.loaded = True
        if len(self.play["chunks"]) > self.args.max_chunks:
            raise wire.ProtocolError("terrain chunk budget exceeded")
        return name, decoded, event

    def initial_play(self) -> None:
        self.phase = "play"
        self.play["status"] = "running"
        deadline = time.monotonic() + self.args.client_timeout
        first_batch_at = None
        while True:
            if self.play["batches"] and first_batch_at is None:
                first_batch_at = time.monotonic()
            ready = (not REQUIRED_STATE.difference(self.seen) and self.loaded and not self.bundle_open
                     and self.batch is None and len(self.play["chunks"]) >= self.args.min_chunks
                     and (self.expected_block is None or self.persisted_block_observed))
            if ready and first_batch_at is not None and time.monotonic() - first_batch_at >= self.args.state_grace:
                profile = self.player_list.get(self.auth["profile_uuid"], {})
                if profile.get("name") != self.args.username:
                    raise wire.ProtocolError("player-info has no current login profile/name")
                self.play["current_player_info"] = profile
                self.play.update(status="pass", first_completed_terrain_batch=True, player_loaded_sent=True,
                                 stop_reason="completed terrain batch + required state + bounded grace",
                                 packet_names=sorted(self.seen))
                return
            next_deadline = deadline
            if ready and first_batch_at is not None:
                next_deadline = min(deadline, first_batch_at + self.args.state_grace)
            try:
                self.receive_play(next_deadline)
            except TimeoutError:
                if next_deadline == deadline or self.socket.buffer:
                    raise

    def until(self, label: str, predicate) -> dict:
        deadline = time.monotonic() + self.args.action_timeout
        while True:
            name, decoded, event = self.receive_play(deadline)
            if predicate(name, decoded):
                result = {"name": label, "status": "pass", "response_sequence": event["sequence"], "response": decoded}
                self.play["checks"].append(result)
                return result

    def command(self, command: str, expected: str) -> dict:
        sent = self.inventory.send(self.socket, "chat_command", wire.string(command))
        result = self.until(f"command /{command}", lambda name, value: name in ("system_chat", "disguised_chat")
                            and any(expected in text for text in wire.text_values(value)))
        result["request_sequence"] = sent["sequence"]
        return result

    def actions(self) -> None:
        self.phase = "actions"
        self.command("version", "26.2")
        self.command("seed", str(self.args.seed))
        self.command("list", self.args.username)
        message = "migration26_2_" + uuid.uuid4().hex[:12]
        self.inventory.send(self.socket, "chat", wire.string(message, 256) + struct.pack(">qq", time.time_ns() // 1_000_000, 0)
                            + b"\x00" + wire.varint(0) + bytes(4))
        self.until("chat echo", lambda name, value: name == "player_chat" and value.get("message") == message
                   and value.get("sender_uuid") == self.auth["profile_uuid"])
        self.command("say " + message, message)
        spawn = self.teleport
        target_x, target_z = spawn["x"] + 16.0, spawn["z"]
        self.inventory.send(self.socket, "move_player_pos_rot", struct.pack(">dddffB", target_x, spawn["y"], target_z, 90, 0, 1))
        target_chunk = {"x": math.floor(target_x / 16), "z": math.floor(target_z / 16)}
        self.until("movement chunk center", lambda name, value: name == "set_chunk_cache_center" and value == target_chunk)
        target_position = (target_chunk["x"], target_chunk["z"])
        self.until("movement terrain completion", lambda name, value: name == "chunk_batch_finished"
                   and target_position in self.cached_chunks and self.batch is None)
        self.inventory.send(self.socket, "ping_request", struct.pack(">q", 262776))
        self.until("play ping echo", lambda name, value: name == "pong_response" and value.get("id") == 262776)
        keepalive_deadline = time.monotonic() + max(self.args.action_timeout, 16)
        if not any(row["name"] == "play_keep_alive_reply" for row in self.play["checks"]):
            while True:
                name, _, event = self.receive_play(keepalive_deadline)
                if name == "keep_alive":
                    self.play["checks"].append({"name": "observed play keepalive", "status": "pass", "sequence": event["sequence"]})
                    break

    def finish(self, error: BaseException | None) -> None:
        for connection in self.connections:
            if connection.outcome.get("closed_unix") is None:
                connection.close()
        if error is not None:
            detail = f"{type(error).__name__}: {error}"
            if self.phase == "login":
                self.auth.update(status="fail", error=detail)
            elif self.phase == "configuration":
                self.config.update(status="fail", error=detail)
            elif self.phase in ("play", "actions", "shutdown"):
                self.play.update(status="fail", error=detail)
            elif self.phase == "version_gates":
                self.auth["version_gate_error"] = detail
        native.write_json(self.capture.output / "status.json", self.status_probe)
        native.write_json(self.capture.output / "auth.json", self.auth)
        native.write_json(self.capture.output / "config.json", self.config)
        native.write_json(self.capture.output / "play.json", self.play)
        self.capture.close({"status": "pass" if error is None else "fail",
                             "error": None if error is None else f"{type(error).__name__}: {error}",
                             "failed_phase": None if error is None else self.phase,
                            "actual_full_client": False})
        if self.gates is not None:
            self.gates.close({"status": "pass" if len(self.auth.get("version_gates", [])) == 2 else "not_complete",
                              "intentional_login_disconnects": True})


def exclusive_output(path: Path) -> Path:
    path = path.resolve()
    target = (native.ROOT / "target").resolve()
    if path.exists() or path == target or not path.is_relative_to(target) or not path.parent.is_dir():
        raise ValueError("output must be absent with an existing parent under target/")
    return path


def server_properties(seed: int, port: int, level_type: str) -> dict[str, str]:
    properties = {"server-ip": "127.0.0.1", "server-port": str(port), "level-name": "world",
                  "level-seed": str(seed), "level-type": "minecraft:" + level_type,
                  "online-mode": "false", "network-compression-threshold": "-1",
                  "view-distance": "2", "simulation-distance": "2", "difficulty": "normal",
                  "gamemode": "creative", "hardcore": "false", "allow-flight": "true",
                  "enable-status": "true", "enable-query": "false", "enable-rcon": "false",
                  "management-server-enabled": "false", "enable-jmx-monitoring": "false",
                  "enforce-secure-profile": "false", "white-list": "false", "spawn-protection": "0",
                  "max-players": "4", "max-tick-time": "-1", "pause-when-empty-seconds": "-1",
                  "code-of-conduct": "false", "accepts-transfers": "false", "resource-pack": "",
                  "initial-enabled-packs": "vanilla", "initial-disabled-packs": "",
                  "motd": "BCore isolated authenticated 26.2 packet fixture"}
    if level_type == "flat":
        properties["generator-settings"] = json.dumps({"biome": "minecraft:plains", "lakes": False,
            "features": False, "layers": [{"block": "minecraft:bedrock", "height": 1},
            {"block": "minecraft:dirt", "height": 2}, {"block": "minecraft:grass_block", "height": 1}],
            "structure_overrides": []}, separators=(",", ":"))
    else:
        properties["generator-settings"] = "{}"
    return properties


def parse_properties(raw: bytes) -> dict[str, str]:
    result = {}
    for line in raw.decode("ascii").splitlines():
        if not line or line.startswith(("#", "!")):
            continue
        key, separator, value = line.partition("=")
        if not separator or key in result:
            raise ValueError("invalid/duplicate Java server property")
        result[key] = value.replace("\\:", ":").replace("\\=", "=").replace("\\\\", "\\")
    return result


def prepare_bcore_world(output: Path, args) -> None:
    provenance = native.ROOT / "target/migration-26_2-registry-data-20261022-01/provenance_26_2.json"
    data = json_object(provenance.read_bytes())
    if (data.get("minecraft"), data.get("protocol"), data.get("data_version"), data.get("jar_sha256")) != (
            "26.2", 776, 4903, native.JAR_SHA256):
        raise ValueError("BCore world-admission registry provenance differs")
    registry_file = provenance.with_name("registries_26_2.json")
    expected = data["generated_sha256"][registry_file.name]
    if native.sha(registry_file) != expected:
        raise ValueError("BCore admission registry file digest differs")
    registry = json_object(registry_file.read_bytes())
    encoded = json.dumps(registry, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode("utf-8")
    fingerprint = digest(encoded)
    if fingerprint != data["registry_fingerprint"]:
        raise ValueError("BCore admission registry fingerprint differs")
    world = output / "world"
    world.mkdir()
    native.write_json(world / "bcore-world.json", {"schema": 1, "minecraft": "26.2", "data_version": 4903,
                      "protocol_version": 776, "registry_fingerprint": list(bytes.fromhex(fingerprint)), "seed": args.seed})
    write_bytes(world / "ops.json", (json.dumps([args.username]) + "\n").encode("utf-8"))
    native.write_json(output / "world-admission.json", {"authority": "authenticated 26.2 registry projection",
                      "provenance": identity(provenance), "registry": identity(registry_file),
                      "metadata": identity(world / "bcore-world.json"), "operator_file": identity(world / "ops.json")})


def isolated_environment(output: Path) -> dict[str, str]:
    environment = {key: value for key in ("SystemRoot", "WINDIR", "COMSPEC", "PATH", "PATHEXT",
                                        "PROCESSOR_ARCHITECTURE", "NUMBER_OF_PROCESSORS")
                   if (value := os.environ.get(key)) is not None}
    for name in ("tmp", "home", "home/AppData/Local", "home/AppData/Roaming"):
        (output / name).mkdir(parents=True, exist_ok=True)
    environment.update(TEMP=str(output / "tmp"), TMP=str(output / "tmp"), HOME=str(output / "home"),
                       USERPROFILE=str(output / "home"), APPDATA=str(output / "home/AppData/Roaming"),
                       LOCALAPPDATA=str(output / "home/AppData/Local"),
                       CARGO_TARGET_DIR=str(native.ROOT / "target/build"), CARGO_BUILD_JOBS="1", RAYON_NUM_THREADS="2")
    return environment


def execute_owned(command: list[str], cwd: Path, output: Path, label: str, environment: dict,
                  timeout: float, log_limit: int) -> dict:
    owner = OwnedProcess(command, cwd, output, label, environment, log_limit)
    owner.start()
    try:
        code = owner.wait(timeout)
        if code:
            raise RuntimeError(f"{label} exited with {code}; see {label}.stderr.log")
    finally:
        cleanup = owner.close(0)
    if cleanup["state"] != "clean" or cleanup["forced"] or cleanup["log_errors"]:
        raise RuntimeError(f"{label} cleanup/log coverage incomplete")
    return cleanup


def prepare_schema(output: Path, environment: dict, args, libraries: list[Path]) -> tuple[PacketInventory, list[str]]:
    build = output / "schema"
    build.mkdir()
    source = output / "helpers/ProtocolSchema.java"
    javac = args.javac.resolve() if args.javac else Path(shutil.which("javac") or "")
    if not javac.is_file():
        raise ValueError("javac is required for the native schema helper; provide --javac")
    native.write_json(output / "javac-input.json", identity(javac.resolve()))
    execute_owned([str(javac), "--release", "21", "-d", str(build), str(source)], build, output,
                  "schema-compile", environment, 120, args.max_log_mib * 1024 * 1024)
    classpath = os.pathsep.join(str(path) for path in (build, native.JAR, *libraries))
    base = [str(native.JAVA), "-Xmx1G", "-XX:ActiveProcessorCount=2", "-Djava.io.tmpdir=" + str(output / "tmp"),
            "-cp", classpath, "ProtocolSchema"]
    inventory_path = output / "packet-inventory.json"
    execute_owned([*base, "inventory", str(inventory_path)], build, output, "schema-inventory", environment,
                  180, args.max_log_mib * 1024 * 1024)
    return PacketInventory(json_object(inventory_path.read_bytes())), base


def start_server(output: Path, environment: dict, args, libraries: list[Path], number: int) -> tuple[OwnedProcess, int, dict]:
    cwd = output / f"server-{number}"
    cwd.mkdir()
    with reserve_port(args.port if number == 1 else 0) as reservation:
        port = reservation.getsockname()[1]
        if args.mode == "official":
            properties = server_properties(args.seed, port, args.level_type)
            text = "".join(f"{key}={value}\n" for key, value in sorted(properties.items()))
            write_bytes(cwd / "server.properties", text.encode("ascii"))
            write_bytes(cwd / "eula.txt", b"eula=true\n")
            native.write_json(output / "server-properties.requested.json", properties)
            classpath = os.pathsep.join(str(path) for path in (native.JAR, *libraries))
            command = [str(native.JAVA), "-Xms256M", "-Xmx1G", "-XX:ActiveProcessorCount=2",
                       "-Djava.io.tmpdir=" + str(output / "tmp"), "-cp", classpath, "net.minecraft.server.Main", "nogui"]
        else:
            command = [str(args.executable), "--world", str(output / "world"), "--seed", str(args.seed),
                       "--host", "127.0.0.1", "--port", str(port)]
        native.write_json(output / f"port-reservation-{number}.json", {
            "host": "127.0.0.1", "port": port, "selection": "exclusive bind", "handoff": "released immediately before Popen",
            "requested": args.port if number == 1 else 0, "no_substitution_on_conflict": True})
        owner = OwnedProcess(command, cwd, output, f"server-{number}", environment, args.max_log_mib * 1024 * 1024)
        reservation.close()
        owner.start()
    try:
        ready = wait_ready(owner, port, args.startup_timeout, native_server=args.mode == "official")
    except BaseException:
        if args.mode == "official" and owner.process.poll() is None:
            try:
                owner.write_stdin(b"stop\n")
            except (OSError, RuntimeError) as error:
                owner.cleanup["attempts"].append({"method": "stdin stop", "error": str(error)})
        owner.close(args.shutdown_timeout, port=port)
        raise
    native.write_json(output / f"listener-{number}.json", ready)
    if args.mode == "official":
        try:
            actual = parse_properties((cwd / "server.properties").read_bytes())
            if any(actual.get(key) != value for key, value in properties.items()):
                raise wire.ProtocolError("effective server properties differ from requested fixture")
            native.write_json(output / "server-properties.effective.json", {"properties": actual, "file": identity(cwd / "server.properties")})
        except BaseException:
            try:
                owner.write_stdin(b"stop\n")
            finally:
                owner.close(args.shutdown_timeout, port=port)
            raise
    return owner, port, ready


def session_run(owner: OwnedProcess, port: int, inventory: PacketInventory, args, output: Path, *, actions: bool,
                gates: bool = False, expected_block: dict | None = None) -> dict:
    session = Session(owner, port, inventory, args, output, expected_block)
    error = None
    try:
        session.play["status_response"] = session.status()
        if gates:
            session.phase = "version_gates"
            session.auth["version_gates"] = [session.version_gate(protocol) for protocol in (775, 777)]
        session.login()
        session.configuration()
        session.initial_play()
        if actions:
            # BCore admits operator names only inside its versioned world root.
            if owner.command[0] != str(args.executable):
                raise RuntimeError("BCore action owner identity differs")
            session.actions()
        return {"sessionUUID": session.auth["sessionUUID"], "chunks": len(session.play["chunks"]),
                "batches": len(session.play["batches"]), "status": "pass", "path": str(output),
                "status_probe": session.status_probe["status"]}
    except BaseException as caught:
        error = caught
        raise
    finally:
        session.finish(error)


def bcore_stop(owner: OwnedProcess, port: int, inventory: PacketInventory, args, output: Path) -> dict:
    stop = Session(owner, port, inventory, args, output)
    error = None
    try:
        stop.login()
        stop.configuration()
        # /stop belongs to the separate teardown transcript, never a fixture.
        stop.phase = "play"
        stop.play["status"] = "running"
        join_deadline = time.monotonic() + args.client_timeout
        while stop.teleport is None:
            stop.receive_play(join_deadline)
        inventory.send(stop.socket, "player_loaded")
        stop.command("list", args.username)
        stop.phase = "shutdown"
        inventory.send(stop.socket, "chat_command", wire.string("stop"))
        deadline = time.monotonic() + args.shutdown_timeout
        while owner.process.poll() is None:
            try:
                stop.receive_play(deadline)
            except EOFError:
                break
            except wire.ProtocolError:
                if stop.capture.sockets[-1].get("disconnect"):
                    break
                raise
            except RuntimeError:
                if owner.process.poll() is not None:
                    break
                raise
        code = owner.wait(max(0.1, deadline - time.monotonic()))
        if code:
            raise RuntimeError(f"BCore /stop exit code {code}")
        stop.play.update(status="pass", intentional_shutdown=True)
        return {"status": "pass", "root_exit_code": code, "method": "operator /stop", "path": str(output)}
    except BaseException as caught:
        error = caught
        raise
    finally:
        stop.finish(error)


def mutate_saved_chunk(output: Path, environment: dict, args) -> dict:
    play = json_object((output / "session-1/play.json").read_bytes())
    position = play["state_packets"]["player_position"]
    cx, cz = math.floor(position["x"] / 16), math.floor(position["z"] / 16)
    command = [str(args.persistence_probe), str(output / "world"), str(args.seed), str(cx), str(cz)]
    execute_owned(command, output, output, "persistence-mutation", environment, 60, args.max_log_mib * 1024 * 1024)
    mutation = json_object((output / "persistence-mutation.stdout.log").read_bytes())
    if (mutation.get("schema"), mutation.get("format_version"), mutation.get("minecraft"),
            mutation.get("protocol"), mutation.get("data_version"), mutation.get("seed"), mutation.get("chunk")) != (
            "bcore.persistence-mutation.v1", 6, "26.2", 776, 4903, str(args.seed), [cx, cz]):
        raise wire.ProtocolError("persistence mutation identity differs")
    expected_path = output / f"world/chunks/c.{cx}.{cz}.bcc"
    if (not Path(mutation["path"]).samefile(expected_path) or mutation["local"] != [7, 0, 7]
            or mutation["block"] != [cx * 16 + 7, 0, cz * 16 + 7]
            or mutation["before_state"] == mutation["state"]
            or mutation["before_sha256"] == mutation["sha256"] or native.sha(expected_path) != mutation["sha256"]):
        raise wire.ProtocolError("persistence mutation did not change the admitted saved chunk")
    mutation["reported_path"] = mutation["path"]
    mutation["path"] = str(expected_path)
    native.write_json(output / "persistence-mutation.json", mutation)
    return mutation


def check_schema_report(validation: dict, sessions: list[Path], *, min_chunks: int | None = None) -> list[dict]:
    if (validation.get("minecraft"), validation.get("protocol")) != ("26.2", 776):
        raise wire.ProtocolError("native schema report identity differs")
    rows = validation.get("sessions")
    if not isinstance(rows, list) or not rows or len(rows) != len(sessions):
        raise wire.ProtocolError("native schema session count differs")
    summaries = []
    for path, row in zip(sessions, rows):
        if Path(row["capture_path"]).resolve() != path.resolve():
            raise wire.ProtocolError("native schema capture path differs")
        packets = row.get("packets")
        if packets is not None and (not isinstance(packets, list) or any(not isinstance(packet, dict) for packet in packets)):
            raise wire.ProtocolError(f"native packet reports malformed for {path.name}")
        failures = [packet for packet in packets or [] if packet.get("status") != "pass"]
        if row["status"] != "pass" or not 0 < row["decoded"] == row["expected"] or failures:
            failed = failures[0] if failures else {}
            detail = failed.get("error") or row.get("error") or "decode incomplete"
            location = (f"sequence {failed.get('sequence')} {failed.get('state')}/{failed.get('direction')} "
                        f"{failed.get('packet', '')} ({failed.get('failure_phase', 'native_validation')})") if failed else path.name
            raise wire.ProtocolError(f"native recorded-frame decode failed for {path.name}: {location}: {detail}")
        if not isinstance(packets, list) or len(packets) != row["expected"]:
            raise wire.ProtocolError(f"native packet report coverage incomplete for {path.name}")
        coverage = row.get("coverage")
        if not isinstance(coverage, dict):
            raise wire.ProtocolError(f"native schema coverage missing for {path.name}")
        if min_chunks is not None:
            required = ("status_response", "status_pong", "login_finished", "login_acknowledged",
                        "network_registry_load", "configuration_acknowledged", "play_login")
            terrain = coverage.get("native_terrain_packets")
            if (any(coverage.get(key) is not True for key in required) or not isinstance(terrain, int)
                    or terrain < min_chunks):
                raise wire.ProtocolError(f"native full-session coverage incomplete for {path.name}")
        summaries.append({"path": str(path), "decoded": row["decoded"], "expected": row["expected"],
                          "coverage": coverage, "scope": "recorded_frames_only"})
    return summaries


def check_schema_inputs(validation: dict, sessions: list[Path], *, min_chunks: int | None = None) -> list[dict]:
    summaries = check_schema_report(validation, sessions, min_chunks=min_chunks)
    for path, row in zip(sessions, validation["sessions"]):
        raw = (path / "wire.json").read_bytes()
        events = json_object(raw)["events"]
        if row["wire_index_sha256"] != digest(raw) or row["expected"] != len(events):
            raise wire.ProtocolError(f"native schema wire index/count differs for {path.name}")
        fields = ("sequence", "state", "direction", "packet_id")
        if any(any(packet[key] != event[key] for key in fields) for packet, event in zip(row["packets"], events)):
            raise wire.ProtocolError(f"native schema packet sequence differs for {path.name}")
        chunks = sum(packet["packet"] == "minecraft:level_chunk_with_light" for packet in row["packets"])
        if row["coverage"]["native_terrain_packets"] != chunks:
            raise wire.ProtocolError(f"native terrain coverage differs from recorded chunks for {path.name}")
    return summaries


def verify_sessions(output: Path, environment: dict, args, base: list[str], sessions: list[Path]) -> list[dict]:
    if not sessions:
        raise wire.ProtocolError("no recorded sessions selected for verification")
    name = "schema-" + "-".join(path.name for path in sessions)
    schema_output = output / f"{name}.json"
    execute_owned([*base, "verify", str(schema_output), *map(str, sessions)], output / "schema", output,
                  name, environment, args.schema_timeout, args.max_log_mib * 1024 * 1024)
    return check_schema_inputs(json_object(schema_output.read_bytes()), sessions, min_chunks=args.min_chunks)


def recorded_inputs(root: Path, libraries: list[Path]) -> tuple[list[Path], dict]:
    inputs = json_object((root / "inputs.json").read_bytes())
    manifest = json_object((root / "manifest.json").read_bytes())
    if (inputs["minecraft"]["name"], inputs["minecraft"]["protocol_version"], inputs["jar"]["sha256"],
            inputs["library_count"]) != ("26.2", 776, native.JAR_SHA256, LIBRARY_COUNT):
        raise wire.ProtocolError("recorded capture target identity differs")
    if sorted(row["sha256"] for row in inputs["libraries"]) != sorted(native.sha(path) for path in libraries):
        raise wire.ProtocolError("recorded capture libraries differ from authenticated environment")
    sessions = sorted(path for path in root.glob("session-*") if path.is_dir() and (path / "wire.json").is_file())
    if not sessions:
        raise wire.ProtocolError("capture has no recorded sessions")
    files = {"inputs.json", *["helpers/" + name for name in inputs["helpers"]]}
    for session in sessions:
        wire_index = json_object((session / "wire.json").read_bytes())
        names = {"wire.json", "auth.json", "config.json", "play.json", "outcome.json", "status.json", "socket-outcomes.json"}
        names.update(event["file"] for event in wire_index["events"])
        if any(Path(name).name != name for name in names):
            raise wire.ProtocolError("recorded frame path is not a capture filename")
        files.update(session.name + "/" + name for name in names)
    receipts = {}
    for name in sorted(files):
        path = root / name
        if path.is_symlink() or not path.resolve().is_relative_to(root):
            raise wire.ProtocolError(f"recorded input escapes capture: {name}")
        actual = identity(path)
        expected = manifest["files"][name]
        if (actual["bytes"], actual["sha256"]) != (expected["bytes"], expected["sha256"]):
            raise wire.ProtocolError(f"recorded input receipt differs: {name}")
        receipts[name] = actual
    for name, receipt in inputs["helpers"].items():
        if receipts["helpers/" + name]["sha256"] != receipt["sha256"]:
            raise wire.ProtocolError(f"recorded helper receipt differs: {name}")
    original = json_object((root / "results.json").read_bytes())
    receipts["results.json"] = identity(root / "results.json")
    receipts["manifest.json"] = identity(root / "manifest.json")
    return sessions, {"path": str(root), "original_status": original["status"], "original_error": original.get("error"),
                      "results": receipts["results.json"], "manifest": receipts["manifest.json"],
                      "files": receipts}


def reverify_run(output: Path, args, version: dict, libraries: list[Path]) -> dict:
    sessions, original = recorded_inputs(args.capture, libraries)
    output.mkdir()
    helpers = output / "helpers"
    helpers.mkdir()
    for source in HELPERS:
        shutil.copyfile(source, helpers / source.name)
        if native.sha(source) != native.sha(helpers / source.name):
            raise RuntimeError(f"source input changed during snapshot: {source.name}")
    inputs = {"operation": "reverify_recorded_protocol", "minecraft": version, "jar": identity(native.JAR),
              "java": identity(native.JAVA), "libraries": [identity(path) for path in libraries],
              "library_count": len(libraries), "preparation": identity(native.PREPARATION),
              "helpers": {source.name: identity(helpers / source.name) for source in HELPERS},
              "original_capture": original, "invocation": [sys.executable, *sys.argv],
              "options": {key: str(value) if isinstance(value, (Path, uuid.UUID)) else value for key, value in vars(args).items()},
              "actual_full_client": False}
    native.write_json(output / "inputs.json", inputs)
    environment = isolated_environment(output)
    result = {"operation": "reverify_recorded_protocol", "status": "running", "actual_full_client": False,
              "original_capture_status": original["original_status"], "schema_validation": "not_attempted",
              "native_session_validation": "not_attempted", "started_unix": time.time()}
    try:
        _, base = prepare_schema(output, environment, args, libraries)
        result["schema_coverage"] = verify_sessions(output, environment, args, base, sessions)
        result.update(status="pass", schema_validation="pass", native_session_validation="pass")
    except BaseException as error:
        result.update(status="fail", schema_validation="fail", native_session_validation="fail",
                      error=f"{type(error).__name__}: {error}")
        write_bytes(output / "failure.txt", traceback.format_exc().encode("utf-8"))
    finally:
        changed, errors = [], {}
        for name, row in original["files"].items():
            try:
                if native.sha(args.capture / name) != row["sha256"]:
                    changed.append(name)
            except OSError as error:
                changed.append(name)
                errors[name] = str(error)
        result["recorded_input_freeze"] = "fail" if changed else "pass"
        result["source_input_freeze"] = verify_source_freeze(output, inputs["helpers"])["status"]
        native.write_json(output / "recorded-input-freeze.json", {"status": result["recorded_input_freeze"],
                          "changed": changed, "errors": errors})
        if changed or result["source_input_freeze"] != "pass":
            result.update(status="fail", input_freeze_error="verification inputs changed during reverify")
    result["finished_unix"] = time.time()
    manifest = {path.relative_to(output).as_posix(): identity(path) for path in output.rglob("*")
                if path.is_file() and not any(part in ("home", "tmp") for part in path.relative_to(output).parts)}
    native.write_json(output / "manifest.json", {"files": manifest, "authority": "independent recorded-byte schema reverify"})
    native.write_json(output / "results.json", result)
    return result


def verify_source_freeze(output: Path, helpers: dict) -> dict:
    expected = {name: row["sha256"] for name, row in helpers.items()}
    observed, errors = {}, {}
    for path in HELPERS:
        try:
            observed[path.name] = native.sha(path)
        except OSError as error:
            observed[path.name] = None
            errors[path.name] = str(error)
    changed = [name for name, value in observed.items() if value != expected[name]]
    receipt = {"status": "fail" if changed else "pass",
               "verification": "completion hashes compared with helper snapshots", "changed": changed,
               "expected_sha256": expected, "observed_sha256": observed, "errors": errors}
    native.write_json(output / "source-freeze.json", receipt)
    return receipt


def executable_freeze(path: Path, expected: dict) -> dict:
    try:
        observed = identity(path)
    except OSError as error:
        return {"status": "fail", "expected": expected, "error": str(error)}
    unchanged = (observed["bytes"], observed["sha256"]) == (expected["bytes"], expected["sha256"])
    return {"status": "pass" if unchanged else "fail", "expected": expected, "observed": observed}


def capture_run(output: Path, args, version: dict, libraries: list[Path]) -> dict:
    output.mkdir()
    helpers = output / "helpers"
    helpers.mkdir()
    for source in HELPERS:
        shutil.copyfile(source, helpers / source.name)
        if native.sha(source) != native.sha(helpers / source.name):
            raise RuntimeError(f"source input changed during snapshot: {source.name}")
    environment = isolated_environment(output)
    inputs = {"schema": 1, "minecraft": version, "jar": identity(native.JAR),
              "preparation": identity(native.PREPARATION), "java": identity(native.JAVA),
              "java_release": identity(native.JAVA.parents[1] / "release"),
              "python": {**identity(Path(sys.executable)), "version": sys.version},
              "helpers": {path.name: identity(helpers / path.name) for path in HELPERS},
              "libraries": [identity(path) for path in libraries], "library_count": len(libraries),
              "invocation": [sys.executable, *sys.argv], "queue_contract": {"CARGO_TARGET_DIR": os.environ.get("CARGO_TARGET_DIR")},
              "options": {key: str(value) if isinstance(value, (Path, uuid.UUID)) else value for key, value in vars(args).items()},
              "actual_full_client": False}
    if args.mode == "bcore":
        inputs["bcore_executable"] = identity(args.executable)
        if args.persistence_probe is not None:
            inputs["persistence_probe"] = identity(args.persistence_probe)
    native.write_json(output / "inputs.json", inputs)
    result = {"status": "running", "mode": args.mode, "actual_full_client": False, "sessions": [], "cleanup": [],
              "schema_validation": "not_attempted", "schema_validation_scope": "recorded_frames_only",
              "native_session_validation": "not_reached", "started_unix": time.time()}
    owner = None
    port = None
    schema_base = None
    try:
        execute_owned([str(native.JAVA), "-version"], output, output, "java-version", environment,
                      30, args.max_log_mib * 1024 * 1024)
        inventory, schema_base = prepare_schema(output, environment, args, libraries)
        if args.mode == "bcore":
            prepare_bcore_world(output, args)
        owner, port, _ = start_server(output, environment, args, libraries, 1)
        result["sessions"].append(session_run(owner, port, inventory, args, output / "session-1",
                                               actions=args.mode == "bcore" and args.exercise == "full",
                                               gates=args.mode == "bcore"))
        result["native_session_validation"] = "running"
        verify_sessions(output, environment, args, schema_base, [output / "session-1"])
        result["schema_validation"] = "pass"
        result["native_session_validation"] = "partial" if args.mode == "bcore" and args.exercise == "full" else "pass"
        if args.mode == "bcore" and args.exercise == "full":
            result["sessions"].append(session_run(owner, port, inventory, args, output / "session-2", actions=False))
            if result["sessions"][0]["sessionUUID"] != result["sessions"][1]["sessionUUID"]:
                raise wire.ProtocolError("same BCore process changed server sessionUUID on reconnect")
            result["native_session_validation"] = "running"
            verify_sessions(output, environment, args, schema_base, [output / "session-2"])
            result["native_session_validation"] = "partial"
            result["shutdown"] = bcore_stop(owner, port, inventory, args, output / "shutdown-1")
            cleanup = owner.close(0, port=port)
            result["cleanup"].append(cleanup)
            if cleanup["state"] != "clean" or cleanup["forced"]:
                raise RuntimeError("BCore graceful shutdown cleanup unknown/forced")
            mutation = mutate_saved_chunk(output, environment, args)
            owner, port, _ = start_server(output, environment, args, libraries, 2)
            result["sessions"].append(session_run(owner, port, inventory, args, output / "session-3", actions=False,
                                                  expected_block=mutation))
            if result["sessions"][0]["sessionUUID"] == result["sessions"][2]["sessionUUID"]:
                raise wire.ProtocolError("restarted BCore reused the previous process sessionUUID")
            result["native_session_validation"] = "running"
            verify_sessions(output, environment, args, schema_base, [output / "session-3"])
            result["native_session_validation"] = "pass"
            result["shutdown_after_reconnect"] = bcore_stop(owner, port, inventory, args, output / "shutdown-2")
            persisted = identity(Path(mutation["path"]))
            if persisted["sha256"] != mutation["sha256"]:
                raise wire.ProtocolError("restarted server changed the persisted probe chunk")
            result["persistence"] = {"status": "pass", "mutation": mutation, "file_after_restart": persisted,
                                     "wire_observation": str(output / "session-3/play.json")}
        if args.mode == "bcore":
            if args.exercise == "smoke":
                result["shutdown"] = bcore_stop(owner, port, inventory, args, output / "shutdown-1")
        result["status"] = "pass"
    except BaseException as error:
        result.update(status="fail", error=f"{type(error).__name__}: {error}")
        if result["native_session_validation"] == "running":
            result["native_session_validation"] = "fail"
        write_bytes(output / "failure.txt", traceback.format_exc().encode("utf-8"))
    finally:
        if owner is not None and owner.cleanup["state"] == "running":
            if args.mode == "official" and owner.process.poll() is None:
                try:
                    owner.write_stdin(b"stop\n")
                except (OSError, RuntimeError) as error:
                    owner.cleanup["attempts"].append({"method": "stdin stop", "error": str(error)})
            cleanup = owner.close(args.shutdown_timeout, port=port)
            result["cleanup"].append(cleanup)
            if cleanup["state"] != "clean" or cleanup["forced"] or cleanup["log_errors"]:
                result.update(status="fail", cleanup_unknown=cleanup["state"] != "clean")
    if schema_base is not None:
        sessions = sorted(path for path in output.glob("session-*") if (path / "wire.json").is_file())
        if sessions:
            try:
                schema_output = output / "schema-validation.json"
                execute_owned([*schema_base, "verify", str(schema_output), *map(str, sessions)], output / "schema", output,
                              "schema-verify", environment, args.schema_timeout, args.max_log_mib * 1024 * 1024)
                validation = json_object(schema_output.read_bytes())
                result["schema_coverage"] = check_schema_report(validation, sessions)
                result["schema_validation"] = "pass"
            except (OSError, ValueError, RuntimeError, TimeoutError) as error:
                result.update(status="fail", schema_validation="fail", schema_error=f"{type(error).__name__}: {error}")
    freeze = verify_source_freeze(output, inputs["helpers"])
    result["source_input_freeze"] = freeze["status"]
    if freeze["status"] != "pass":
        result.update(status="fail", source_input_error=f"source inputs changed during capture: {', '.join(freeze['changed'])}")
    if args.mode == "bcore":
        result["bcore_executable_freeze"] = executable_freeze(args.executable, inputs["bcore_executable"])
        if result["bcore_executable_freeze"]["status"] != "pass":
            result.update(status="fail", executable_input_error="BCore executable changed during capture")
        if args.persistence_probe is not None:
            result["persistence_probe_freeze"] = executable_freeze(args.persistence_probe, inputs["persistence_probe"])
            if result["persistence_probe_freeze"]["status"] != "pass":
                result.update(status="fail", executable_input_error="persistence probe changed during capture")
    result["finished_unix"] = time.time()
    manifest = {path.relative_to(output).as_posix(): identity(path) for path in output.rglob("*")
                if path.is_file() and not any(part in ("world", "home", "tmp") for part in path.relative_to(output).parts)}
    native.write_json(output / "manifest.json", {"files": manifest, "fixture_authority": "fresh owned socket transcript",
                                              "binary_format": "big-endian u32 count; repeated i32 packet_id, u32 length, payload"})
    native.write_json(output / "results.json", result)
    return result


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("official", "bcore", "reverify"))
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--capture", type=Path, help="reverify only: existing capture root; writes a new schema result")
    parser.add_argument("--executable", type=Path, help="already-built BCore executable; no Cargo invocation")
    parser.add_argument("--persistence-probe", type=Path, help="full BCore probe: built persistence_probe.exe; edits only the owned save")
    parser.add_argument("--exercise", choices=("smoke", "full"), default="smoke")
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument("--level-type", choices=("flat", "normal"), default="flat")
    parser.add_argument("--port", type=int, default=0, help="0 selects an exclusively reserved ephemeral loopback port")
    parser.add_argument("--username", default="BCore262Probe")
    parser.add_argument("--player-uuid", type=uuid.UUID, default=uuid.UUID("3a262776-7dd6-4d12-b589-86aa55ee1188"))
    parser.add_argument("--radius", type=int, default=2)
    parser.add_argument("--chunks-per-tick", type=float, default=2.0)
    parser.add_argument("--startup-timeout", type=float, default=240)
    parser.add_argument("--client-timeout", type=float, default=90)
    parser.add_argument("--socket-timeout", type=float, default=10)
    parser.add_argument("--status-retry-settle", type=float, default=0.5,
                        help="official only: one exact status retry after a bounded settle, retaining the first failure")
    parser.add_argument("--action-timeout", type=float, default=20)
    parser.add_argument("--shutdown-timeout", type=float, default=60)
    parser.add_argument("--schema-timeout", type=float, default=240)
    parser.add_argument("--state-grace", type=float, default=0.5)
    parser.add_argument("--min-chunks", type=int, default=1)
    parser.add_argument("--max-chunks", type=int, default=128)
    parser.add_argument("--max-packets", type=int, default=4096)
    parser.add_argument("--max-capture-mib", type=int, default=128)
    parser.add_argument("--max-log-mib", type=int, default=32)
    parser.add_argument("--javac", type=Path)
    args = parser.parse_args(argv)
    try:
        args.output = exclusive_output(args.output)
        if args.mode == "reverify":
            if args.capture is None:
                raise ValueError("reverify requires --capture")
            args.capture = args.capture.resolve(strict=True)
            if not args.capture.is_dir() or not args.capture.is_relative_to((native.ROOT / "target").resolve()):
                raise ValueError("recorded capture must be an existing directory under target/")
            if args.output.is_relative_to(args.capture):
                raise ValueError("reverify output must be outside the immutable capture directory")
        elif args.capture is not None:
            raise ValueError("--capture requires reverify mode")
        if not -(1 << 63) <= args.seed < (1 << 63) or not 0 <= args.port <= 65535:
            raise ValueError("seed/port outside native wire ranges")
        if args.radius != 2:
            raise ValueError("fixture/probe radius must be 2")
        wire.string(args.username, 16)
        if not args.username or not args.username.isascii() or not all(char.isalnum() or char == "_" for char in args.username):
            raise ValueError("offline username must contain 1..16 ASCII letters/digits/underscores")
        for name in ("startup_timeout", "client_timeout", "socket_timeout", "action_timeout", "shutdown_timeout", "schema_timeout"):
            if not math.isfinite(getattr(args, name)) or not 0 < getattr(args, name) <= 1800:
                raise ValueError(f"{name} must be finite and within (0, 1800]")
        if not math.isfinite(args.state_grace) or not 0 <= args.state_grace <= 5:
            raise ValueError("state grace must be within 0..5 seconds")
        if not math.isfinite(args.status_retry_settle) or not 0 < args.status_retry_settle <= 5:
            raise ValueError("status retry settle must be finite within (0, 5] seconds")
        if not math.isfinite(args.chunks_per_tick) or not 0.01 <= args.chunks_per_tick <= 64:
            raise ValueError("desired chunks/tick must be finite within 0.01..64")
        if not 1 <= args.min_chunks <= args.max_chunks <= 4096 or not 32 <= args.max_packets <= 100000:
            raise ValueError("invalid chunk/packet budgets")
        if not 1 <= args.max_capture_mib <= 1024 or not 1 <= args.max_log_mib <= 256:
            raise ValueError("invalid capture/log byte budgets")
        if args.mode == "bcore":
            if args.executable is None:
                raise ValueError("BCore mode requires --executable")
            args.executable = args.executable.resolve(strict=True)
            if not args.executable.is_file() or args.executable.suffix.lower() != ".exe":
                raise ValueError("BCore executable must be an existing .exe")
            if args.exercise == "full":
                if args.persistence_probe is None:
                    raise ValueError("full BCore exercise requires --persistence-probe")
                args.persistence_probe = args.persistence_probe.resolve(strict=True)
                if not args.persistence_probe.is_file() or args.persistence_probe.suffix.lower() != ".exe":
                    raise ValueError("persistence probe must be an existing .exe")
            elif args.persistence_probe is not None:
                raise ValueError("--persistence-probe requires full BCore exercise")
        elif args.executable is not None or args.exercise != "smoke":
            raise ValueError("--executable/--exercise full require BCore mode")
        elif args.persistence_probe is not None:
            raise ValueError("--persistence-probe requires full BCore exercise")
    except (OSError, ValueError) as error:
        parser.error(str(error))
    return args


def main() -> None:
    args = parse_args()
    version, libraries = native.environment()
    if native.JAR_SHA256 != "183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8" or len(libraries) != LIBRARY_COUNT:
        raise ValueError("capture requires the exact 26.2 JAR and 39 authenticated bundle libraries")
    result = reverify_run(args.output, args, version, libraries) if args.mode == "reverify" else capture_run(args.output, args, version, libraries)
    print(json.dumps({"output": str(args.output), **result}, indent=2), flush=True)
    if result["status"] != "pass":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
