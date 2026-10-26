"""Repeat-authenticate native pillar placement and publish its byte-exact fixture."""
import argparse
import json
from pathlib import Path

import native


def capture(output: Path) -> dict:
    version, libraries = native.environment()
    output.mkdir(exist_ok=False)
    sources = [native.ROOT / "scripts/TreeReference.java", native.ROOT / "scripts/NativeWorldgenRegistries.java",
               native.HERE / "ProtocolSchema.java", native.HERE / "BlockActionReference.java",
               native.HERE / "PillarPlacementReference.java"]
    before = {path.relative_to(native.ROOT).as_posix(): native.sha(path) for path in sources}
    first = native.capture_probe(output, libraries, "run-1", sources)
    second = native.capture_probe(output, libraries, "run-2", sources)
    if first != second or before != {path.relative_to(native.ROOT).as_posix(): native.sha(path) for path in sources}:
        raise ValueError("native pillar repeat or helper source identity differs")
    items = first["axis_full_cube_block_items"]
    cases = first["cases"]
    if first["schema"] != "bcore.native-pillar-placement.v1" or not items or len(cases) != 6 * len(items):
        raise ValueError("unexpected native pillar case inventory")
    if len({row["item_id"] for row in items}) != len(items):
        raise ValueError("duplicate native pillar item")
    for item in items:
        placements = [row for row in cases if row["input"]["item_id"] == item["item_id"]]
        if set(item["states"]) != {"x", "y", "z"} or {row["input"]["face"] for row in placements} != set(range(6)):
            raise ValueError("incomplete native pillar axes/faces")
        if any(row["input"]["state_id"] != item["states"][row["input"]["axis"]] for row in placements):
            raise ValueError("placement state differs from native inventory")
    if not all(row["native_decode_consumed_all"] and row["packet"] == "minecraft:block_update"
               and row["protocol_id"] == 8 for row in cases):
        raise ValueError("incomplete native pillar block-update decode")
    metadata = {
        "schema": "bcore.native-pillar-placement-capture.v1", "version": version,
        "jar_sha256": native.JAR_SHA256, "java_sha256": native.sha(native.JAVA),
        "libraries_sha256": {path.relative_to(native.ROOT).as_posix(): native.sha(path) for path in libraries},
        "source_sha256": before, "repeat_matches": True, "cases": len(cases), "block_items": len(items),
        "state_count": len({state for row in items for state in row["states"].values()}),
        "run_sha256": {name: native.sha(output / (name + ".json")) for name in ("run-1", "run-2")},
    }
    native.write_json(output / "results.json", metadata)
    return metadata


def publish(output: Path, metadata: dict) -> None:
    data = native.ROOT / "crates/bcore-protocol/data"
    fixture = data / "pillar_placement_26_2.json"
    provenance = data / "pillar_placement_provenance_26_2.json"
    if fixture.exists() or provenance.exists() or not data.is_dir():
        raise ValueError("native pillar fixture publication must be create-new in protocol data")
    with fixture.open("xb") as stream:
        stream.write((output / "run-1.json").read_bytes())
    native.write_json(provenance, {
        **metadata, "raw_capture": (output / "run-1.json").relative_to(native.ROOT).as_posix(),
        "raw_capture_sha256": native.sha(fixture), "fixture_bytes": fixture.stat().st_size,
    })


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--publish", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(native.ROOT / "target") or output.exists():
        parser.error("output must be a new ROOT/target directory")
    metadata = capture(output)
    if args.publish:
        publish(output, metadata)
    print(json.dumps({"output": str(output), "repeat_matches": True, "cases": metadata["cases"],
                      "block_items": metadata["block_items"], "states": metadata["state_count"],
                      "published": args.publish}), flush=True)


if __name__ == "__main__":
    main()
