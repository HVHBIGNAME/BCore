"""Audit packet blobs against authenticated, native-decoded 26.2 captures.

Run through scripts/run_generation_check.py. Inspection does not change captures
or published data; packet IDs, payload bytes and record order are compared exactly.
"""
from __future__ import annotations

import argparse
from collections import Counter
from pathlib import Path
import json
import struct

import native
import protocol_capture as capture
import protocol_wire as wire

DATA = native.ROOT / "crates/bcore-protocol/data"
FILES = {"configuration": "config_packets.bin", "play": "play_packets.bin"}


def packets(raw: bytes) -> list[tuple[int, bytes]]:
    if len(raw) < 4:
        raise wire.ProtocolError("packet blob lacks its count")
    count = struct.unpack_from(">I", raw)[0]
    if count > 65536:
        raise wire.ProtocolError("packet blob count exceeds budget")
    rows, at = [], 4
    for _ in range(count):
        if at + 8 > len(raw):
            raise wire.ProtocolError("packet blob header is truncated")
        packet_id, size = struct.unpack_from(">iI", raw, at)
        at += 8
        if packet_id < 0 or size > wire.MAX_FRAME or at + size > len(raw):
            raise wire.ProtocolError("packet blob ID/length is invalid")
        rows.append((packet_id, raw[at:at + size]))
        at += size
    if at != len(raw):
        raise wire.ProtocolError("packet blob has trailing bytes")
    return rows


def checked_file(root: Path, name: str, manifest: dict) -> bytes:
    path = root / name
    if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(root):
        raise wire.ProtocolError(f"packet evidence escapes its root: {name}")
    raw = path.read_bytes()
    row = manifest["files"][name]
    if (len(raw), capture.digest(raw)) != (row["bytes"], row["sha256"]):
        raise wire.ProtocolError(f"packet evidence receipt differs: {name}")
    return raw


def native_proof(root: Path, source: Path, sessions: list[Path], source_receipt: dict) -> dict:
    manifest = capture.json_object((root / "manifest.json").read_bytes())
    result = capture.json_object((root / "results.json").read_bytes())
    required = ("status", "schema_validation", "native_session_validation", "source_input_freeze")
    if any(result.get(key) != "pass" for key in required):
        raise wire.ProtocolError("native packet validation did not pass")
    if root == source:
        if result.get("mode") != "official" or any(
                row.get("state") != "clean" or row.get("forced") or row.get("log_errors")
                for row in result.get("cleanup", [])) or not result.get("cleanup"):
            raise wire.ProtocolError("official capture owner/cleanup differs")
        report_name = "schema-validation.json"
    else:
        inputs = capture.json_object(checked_file(root, "inputs.json", manifest))
        previous = inputs["original_capture"]
        if (result.get("operation") != "reverify_recorded_protocol" or result.get("recorded_input_freeze") != "pass"
                or Path(previous["path"]).resolve() != source
                or previous["manifest"]["sha256"] != source_receipt["manifest"]["sha256"]
                or previous["results"]["sha256"] != source_receipt["results"]["sha256"]):
            raise wire.ProtocolError("native reverify is not bound to this capture")
        report_name = "schema-" + "-".join(path.name for path in sessions) + ".json"
    report = capture.json_object(checked_file(root, report_name, manifest))
    coverage = capture.check_schema_inputs(report, sessions, min_chunks=1)
    return {"path": str(root), "manifest": capture.identity(root / "manifest.json"),
            "results": capture.identity(root / "results.json"), "report": capture.identity(root / report_name),
            "coverage": coverage}


def captured(source: Path, libraries: list[Path], validation: Path | None) -> tuple[dict, dict, dict]:
    sessions, receipt = capture.recorded_inputs(source, libraries)
    inputs = capture.json_object((source / "inputs.json").read_bytes())
    if inputs["options"]["mode"] != "official" or len(sessions) != 1:
        raise wire.ProtocolError("one official capture session is required")
    proof = native_proof(validation or source, source, sessions, receipt)
    manifest = capture.json_object((source / "manifest.json").read_bytes())
    inventory = capture.PacketInventory(capture.json_object(checked_file(source, "packet-inventory.json", manifest)))
    session = sessions[0]
    events = capture.json_object((session / "wire.json").read_bytes())["events"]
    blobs, rows = {}, {}
    for state, filename in FILES.items():
        raw = checked_file(source, session.name + "/" + filename, manifest)
        observed = packets(raw)
        recorded = []
        streams = {}
        for event in events:
            if (event["state"], event["direction"]) != (state, "clientbound"):
                continue
            name = event["file"]
            if name not in streams:
                streams[name] = checked_file(source, session.name + "/" + name, manifest)
            framed = streams[name][event["offset"]:event["offset"] + event["frame_length"]]
            decoded = wire.split_frame(framed)
            if decoded is None or decoded[0] != len(framed) or capture.digest(framed) != event["frame_sha256"]:
                raise wire.ProtocolError("recorded packet frame differs")
            _, packet_id, payload = decoded
            if (packet_id, len(payload), capture.digest(payload)) != (
                    event["packet_id"], event["payload_length"], event["payload_sha256"]):
                raise wire.ProtocolError("recorded packet payload differs")
            recorded.append((packet_id, payload))
        if observed != recorded:
            raise wire.ProtocolError(f"{filename} differs from its exact native frame transcript")
        blobs[state] = raw
        rows[state] = [{"index": index, "packet_id": packet_id, "packet": inventory.name(state, packet_id),
                        "bytes": len(payload), "sha256": capture.digest(payload)}
                       for index, (packet_id, payload) in enumerate(observed)]
    return blobs, rows, {"capture": receipt, "native_validation": proof, "inputs": capture.identity(source / "inputs.json")}


def compare(left: bytes, right: bytes) -> dict:
    before, after = packets(left), packets(right)
    difference = next((index for index, pair in enumerate(zip(before, after)) if pair[0] != pair[1]), None)
    if difference is None and len(before) != len(after):
        difference = min(len(before), len(after))
    return {"equal": left == right, "left_count": len(before), "right_count": len(after),
            "left_sha256": capture.digest(left), "right_sha256": capture.digest(right),
            "first_different_record": difference}


def configuration_values(raw: bytes, inventory: capture.PacketInventory) -> list[dict]:
    decoders = {"registry_data": wire.registry_data, "select_known_packs": wire.known_packs,
                "update_enabled_features": wire.enabled_features, "update_tags": wire.tags}
    result = []
    for packet_id, payload in packets(raw):
        name = inventory.name("configuration", packet_id)
        if name in decoders:
            value = decoders[name](payload)
        elif name == "finish_configuration":
            if payload:
                raise wire.ProtocolError("configuration finish payload is not empty")
            value = None
        elif name == "custom_payload":
            reader = wire.Reader(payload)
            channel = reader.identifier()
            if channel != "minecraft:brand":
                raise wire.ProtocolError(f"unsupported configuration custom payload {channel}")
            value = {"channel": channel, "brand": reader.string()}
            reader.finish()
        else:
            raise wire.ProtocolError(f"unsupported configuration packet {name}")
        result.append({"packet": name, "value": value})
    return result


def compare_configuration(left: bytes, right: bytes, inventory: capture.PacketInventory) -> dict:
    before, after = configuration_values(left, inventory), configuration_values(right, inventory)
    changed = [index for index, (old, new) in enumerate(zip(before, after)) if old != new]
    changed.extend(range(min(len(before), len(after)), max(len(before), len(after))))
    return {"equal": before == after, "changed_records": changed,
            "contract": "ordered packet IDs, registry entries, tags and typed NBT; compound member order is not semantic"}


def registry_bindings(raw: bytes, inventory: capture.PacketInventory) -> dict:
    registries = {row["value"]["registry"]: row["value"]["entries"]
                  for row in configuration_values(raw, inventory) if row["packet"] == "registry_data"}
    shared_path = native.ROOT / "crates/bcore-registry/data/registries_26_2.json"
    shared = capture.json_object(shared_path.read_bytes())
    if (shared["minecraft"], shared["protocol"], shared["data_version"], shared["jar_sha256"]) != (
            "26.2", 776, 4903, native.JAR_SHA256):
        raise wire.ProtocolError("shared registry target identity differs")
    biomes = [{"id": row["id"], "key": row["key"]} for row in registries["minecraft:worldgen/biome"]]
    if biomes != shared["registries"]["BIOME"]:
        raise wire.ProtocolError("network biome IDs differ from the shared registry")
    dimensions = [{"id": row["id"], "key": row["key"]} for row in registries["minecraft:dimension_type"]]
    return {"shared_registry": capture.identity(shared_path), "biome_ids_match": True,
            "biome_count": len(biomes), "dimension_types": dimensions}


def require_bcore_result(result: dict, inputs: dict) -> None:
    if (result.get("mode"), inputs["options"]["mode"], inputs["options"]["exercise"]) != ("bcore", "bcore", "full"):
        raise wire.ProtocolError("bootstrap publication requires the full BCore probe")
    if any(result.get(key) != "pass" for key in ("status", "schema_validation", "native_session_validation", "source_input_freeze")):
        raise wire.ProtocolError("BCore native validation did not pass")
    if any(result.get(key, {}).get("status") != "pass" for key in ("bcore_executable_freeze", "persistence_probe_freeze", "persistence")):
        raise wire.ProtocolError("BCore executable/persistence proof is incomplete")
    cleanup = result.get("cleanup", [])
    if len(cleanup) != 2 or any(row.get("state") != "clean" or row.get("forced") is not False
            or row.get("root_exit_code") != 0 or row.get("job_active_before_close") != 0
            or row.get("listeners_after") != [] or row.get("errors") != [] or row.get("log_errors") != []
            or row.get("port_exclusively_rebound") is not True for row in cleanup):
        raise wire.ProtocolError("BCore restart cleanup is not complete and owned")
    for key in ("shutdown", "shutdown_after_reconnect"):
        row = result.get(key, {})
        if (row.get("status"), row.get("root_exit_code"), row.get("method")) != ("pass", 0, "operator /stop"):
            raise wire.ProtocolError("BCore graceful shutdown proof differs")


def bcore_proof(root: Path, release_path: Path, libraries: list[Path]) -> dict:
    sessions, receipt = capture.recorded_inputs(root, libraries)
    inputs = capture.json_object((root / "inputs.json").read_bytes())
    result = capture.json_object((root / "results.json").read_bytes())
    require_bcore_result(result, inputs)
    if len(sessions) != 3 or len(result["sessions"]) != 3:
        raise wire.ProtocolError("BCore probe must contain all three sessions")
    uuids = [capture.json_object((session / "auth.json").read_bytes())["sessionUUID"] for session in sessions]
    if uuids[0] != uuids[1] or uuids[0] == uuids[2]:
        raise wire.ProtocolError("BCore process session identity differs")
    manifest = capture.json_object((root / "manifest.json").read_bytes())
    report = capture.json_object(checked_file(root, "schema-validation.json", manifest))
    coverage = capture.check_schema_inputs(report, sessions, min_chunks=1)
    release = capture.json_object(release_path.read_bytes())
    if (release.get("schema"), release.get("minecraft", {}).get("name"), release.get("jar_sha256")) != (
            "bcore.release-freeze.v1", "26.2", native.JAR_SHA256):
        raise wire.ProtocolError("frozen release target identity differs")
    for name in ("executable", "persistence_probe"):
        key = "bcore_executable" if name == "executable" else name
        actual = capture.identity(Path(inputs[key]["path"]))
        if any(actual[field] != release[name][field] or actual[field] != inputs[key][field] for field in ("bytes", "sha256")):
            raise wire.ProtocolError(f"BCore frozen {name} differs")
    if native.sha(release_path.parent / "source.zip") != release["sources"]["source_archive_sha256"]:
        raise wire.ProtocolError("frozen release source archive changed")
    if native.sha(release_path.parent / "source-manifest.json") != release["sources"]["source_manifest_sha256"]:
        raise wire.ProtocolError("frozen release source manifest changed")
    for filename in FILES.values():
        key = "crates/bcore-protocol/data/" + filename
        if native.sha(DATA / filename) != release["sources"]["source_sha256"][key]:
            raise wire.ProtocolError("published bootstrap differs from the validated executable inputs")
    config = (DATA / FILES["configuration"]).read_bytes()
    for session in sessions:
        if checked_file(root, session.name + "/config_packets.bin", manifest) != config:
            raise wire.ProtocolError("BCore emitted configuration differs from the published bytes")
    persistence = result["persistence"]
    if native.sha(Path(persistence["mutation"]["path"])) != persistence["mutation"]["sha256"]:
        raise wire.ProtocolError("persisted probe chunk changed after validation")
    return {"capture_manifest": receipt["manifest"], "capture_results": receipt["results"],
            "release": capture.identity(release_path), "executable_sha256": inputs["bcore_executable"]["sha256"],
            "coverage": coverage, "session_uuids": uuids, "persistence": persistence,
            "schema_report": capture.identity(root / "schema-validation.json")}


def publish_bootstrap(result: dict, destination: Path | None = None) -> dict:
    if (result.get("published_configuration_values", {}).get("equal") is not True
            or result.get("repeat_configuration_values", {}).get("equal") is not True
            or result.get("registry_bindings", {}).get("biome_ids_match") is not True
            or "bcore_validation" not in result):
        raise wire.ProtocolError("bootstrap publication requires both native configuration captures and BCore validation")
    value = {"schema": "bcore.protocol-bootstrap.v1", "minecraft": "26.2", "protocol": 776,
             "data_version": 4903, "jar_sha256": native.JAR_SHA256,
             "files": {filename: {**capture.identity(DATA / filename), "packet_count": len(packets((DATA / filename).read_bytes()))}
                       for filename in FILES.values()},
             "configuration_comparison": result["published_configuration_values"],
             "official_captures": [result["source"]["capture"]["manifest"], result["repeat"]["capture"]["manifest"]],
             "registry_bindings": result["registry_bindings"], "bcore_validation": result["bcore_validation"],
             "play_authority": "emitted replay and owner-generated packets decoded through native 26.2 registry-aware codecs"}
    destination = (DATA / "protocol_26_2.json" if destination is None else destination).resolve()
    if (destination.suffix != ".json" or not destination.parent.is_dir()
            or destination.parent not in (DATA.resolve(), (native.ROOT / "docs/metrics").resolve())):
        raise wire.ProtocolError("bootstrap publication must be JSON in protocol data or docs/metrics")
    raw = (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n").encode("utf-8")
    if destination.exists():
        if destination.read_bytes() != raw:
            raise wire.ProtocolError("different bootstrap publication already exists")
    else:
        with destination.open("xb") as stream:
            stream.write(raw)
    if destination.read_bytes() != raw:
        raise wire.ProtocolError("bootstrap publication readback differs")
    return capture.identity(destination)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--validation", type=Path)
    parser.add_argument("--repeat", type=Path)
    parser.add_argument("--repeat-validation", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--publish", action="store_true")
    parser.add_argument("--publication-output", type=Path, help="versioned JSON in protocol data or docs/metrics; differing existing bytes are refused")
    parser.add_argument("--bcore-validation", type=Path)
    parser.add_argument("--release", type=Path)
    args = parser.parse_args()
    if args.publication_output is not None and not args.publish:
        parser.error("--publication-output requires --publish")
    output = args.output.resolve()
    if output.exists() or not output.parent.is_dir() or not output.is_relative_to(native.ROOT / "target"):
        parser.error("output must be absent with an existing parent under target/")
    if args.repeat_validation is not None and args.repeat is None:
        parser.error("--repeat-validation requires --repeat")
    if args.publish and (args.repeat is None or args.bcore_validation is None or args.release is None):
        parser.error("--publish requires --repeat, --bcore-validation and --release")
    if bool(args.bcore_validation) != bool(args.release):
        parser.error("--bcore-validation and --release must be supplied together")
    _, libraries = native.environment()
    source = args.source.resolve()
    blobs, rows, evidence = captured(source, libraries, args.validation.resolve() if args.validation else None)
    inventory = capture.PacketInventory(capture.json_object((source / "packet-inventory.json").read_bytes()))
    existing_config = (DATA / FILES["configuration"]).read_bytes()
    result = {"schema": "bcore.protocol-data-audit.v1", "minecraft": "26.2", "protocol": 776,
              "jar_sha256": native.JAR_SHA256, "source": evidence, "packets": rows,
              "published": {state: compare((DATA / filename).read_bytes(), blobs[state]) for state, filename in FILES.items()},
              "packet_counts": {state: dict(Counter(row["packet"] for row in entries)) for state, entries in rows.items()}}
    result["published_configuration_values"] = compare_configuration(existing_config, blobs["configuration"], inventory)
    result["registry_bindings"] = registry_bindings(blobs["configuration"], inventory)
    if args.repeat:
        repeated, _, repeat_evidence = captured(args.repeat.resolve(), libraries,
                                               args.repeat_validation.resolve() if args.repeat_validation else None)
        result["repeat"] = repeat_evidence
        result["repeat_comparison"] = {state: compare(blobs[state], repeated[state]) for state in FILES}
        result["repeat_configuration_values"] = compare_configuration(blobs["configuration"], repeated["configuration"], inventory)
    if args.bcore_validation is not None:
        result["bcore_validation"] = bcore_proof(args.bcore_validation.resolve(), args.release.resolve(), libraries)
    if args.publish:
        result["publication"] = publish_bootstrap(result, args.publication_output)
    native.write_json(output, result)
    print(json.dumps({key: result[key] for key in ("schema", "published", "packet_counts",
                                                 "published_configuration_values", "registry_bindings")}, indent=2))
    if args.repeat:
        print(json.dumps({key: result[key] for key in ("repeat_comparison", "repeat_configuration_values")}, indent=2))
    if args.publish:
        print(json.dumps({"publication": result["publication"]}, indent=2))


if __name__ == "__main__":
    main()
