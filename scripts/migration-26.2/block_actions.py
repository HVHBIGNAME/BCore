"""Repeat-authenticate native block codecs and publish a byte-exact bounded fixture."""
import argparse
import json
from pathlib import Path

import native


def capture(output: Path) -> tuple[dict, dict]:
    version, libraries = native.environment()
    output.mkdir(exist_ok=False)
    sources = [native.ROOT / "scripts/TreeReference.java", native.ROOT / "scripts/NativeWorldgenRegistries.java",
               native.HERE / "ProtocolSchema.java", native.HERE / "BlockActionReference.java"]
    before = {path.relative_to(native.ROOT).as_posix(): native.sha(path) for path in sources}
    first = native.capture_probe(output, libraries, "run-1", sources)
    second = native.capture_probe(output, libraries, "run-2", sources)
    if first != second or before != {path.relative_to(native.ROOT).as_posix(): native.sha(path) for path in sources}:
        raise ValueError("native repeat or helper source identity differs")
    if first["schema"] != "bcore.native-block-actions.v1" or len(first["cases"]) != 25:
        raise ValueError("unexpected native codec case inventory")
    if len(first["stateless_full_cube_block_items"]) != 197:
        raise ValueError("unexpected native block-item inventory")
    if not all(row["native_decode_consumed_all"] for row in first["cases"]):
        raise ValueError("incomplete native payload decode")
    metadata = {
        "schema": "bcore.native-block-actions-capture.v1", "version": version,
        "jar_sha256": native.JAR_SHA256, "java_sha256": native.sha(native.JAVA),
        "libraries_sha256": {path.relative_to(native.ROOT).as_posix(): native.sha(path) for path in libraries},
        "source_sha256": before, "repeat_matches": True,
        "cases": len(first["cases"]), "block_items": len(first["stateless_full_cube_block_items"]),
        "run_sha256": {name: native.sha(output / (name + ".json")) for name in ("run-1", "run-2")},
    }
    native.write_json(output / "results.json", metadata)
    return first, metadata


def publish(output: Path, metadata: dict) -> None:
    data = native.ROOT / "crates/bcore-protocol/data"
    fixture = data / "block_actions_26_2.json"
    provenance = data / "block_actions_provenance_26_2.json"
    if fixture.exists() or provenance.exists():
        raise ValueError("native fixture publication must be create-new")
    if not data.is_dir():
        raise ValueError("protocol data directory is missing")
    raw = (output / "run-1.json").read_bytes()
    with fixture.open("xb") as stream:
        stream.write(raw)
    native.write_json(provenance, {**metadata, "raw_capture": (output / "run-1.json").relative_to(native.ROOT).as_posix(),
                                  "raw_capture_sha256": native.sha(fixture), "fixture_bytes": len(raw)})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--publish", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(native.ROOT / "target") or output.exists():
        parser.error("output must be a new ROOT/target directory")
    _, metadata = capture(output)
    if args.publish:
        publish(output, metadata)
    print(json.dumps({"output": str(output), "repeat_matches": True, "cases": metadata["cases"],
                      "block_items": metadata["block_items"], "published": args.publish}), flush=True)


if __name__ == "__main__":
    main()
