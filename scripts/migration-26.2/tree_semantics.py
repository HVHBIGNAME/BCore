"""Capture and promote bounded target tree/provider observations through the shared queue."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys

import catalogs
import native

SOURCES = (*catalogs.VEGETATION, "scripts/ExtraTreeReference.java",
           "scripts/migration-26.2/TreeSemanticsReference.java")
HELPERS = ("scripts/migration-26.2/tree_semantics.py", "scripts/migration-26.2/native.py",
           "scripts/migration-26.2/catalogs.py")
FIXTURE = "tree_semantics_26_2.json"
MARKER = "TREE_SEMANTICS_REFERENCE="


def verified(source: Path, pin: str) -> tuple[dict, dict]:
    if native.sha(source / "manifest.json") != pin:
        raise ValueError("tree capture manifest changed")
    manifest = json.loads((source / "manifest.json").read_bytes())
    if manifest["jar_sha256"] != native.JAR_SHA256:
        raise ValueError("tree capture belongs to a different native target")
    for name, expected in manifest["files_sha256"].items():
        path = source / name
        if not path.resolve().is_relative_to(source) or native.sha(path) != expected:
            raise ValueError(f"tree capture input changed: {name}")
    result = json.loads((source / FIXTURE).read_bytes())
    if (result["minecraft"], result["protocol"], result["jar_sha256"]) != ("26.2", 776, native.JAR_SHA256):
        raise ValueError("tree fixture identity differs")
    return manifest, result


def capture(output: Path, repeat: Path | None, pin: str | None) -> None:
    version, libraries = native.environment()
    javac = shutil.which("javac")
    if javac is None or len(libraries) != 39:
        raise ValueError("javac and all 39 authenticated target libraries are required")
    if repeat is not None:
        previous_manifest, previous = verified(repeat, pin)
    output.mkdir()
    sources = {name: catalogs.snapshot(native.ROOT / name, output / "sources" / name)
               for name in (*SOURCES, *HELPERS)}
    historical = catalogs.preserved_source_hashes(sources)
    if repeat is not None and sources != previous_manifest["source_sha256"]:
        raise ValueError("independent tree repeat sources differ")
    evidence = catalogs.capture_environment(output, libraries, version, javac, None, None, 64 * 1024 * 1024)
    environment = dict(os.environ)
    environment.update({key: str(output / "tmp") for key in ("TMP", "TEMP", "TMPDIR")})

    def execute(command, name, *, timeout):
        return catalogs.bounded_execute(command, output, name, timeout=timeout,
                                        log_limit=64 * 1024 * 1024, environment=environment)

    build = catalogs.compile_probe(catalogs.Probe("TreeSemanticsReference", SOURCES, FIXTURE),
                                   output, javac, None, execute)
    stdout = execute([str(native.JAVA), "-Xmx1G", "-XX:ActiveProcessorCount=2",
                      f"-Djava.io.tmpdir={output / 'tmp'}", "-cp",
                      os.pathsep.join(map(str, [build, native.JAR, *libraries])),
                      "TreeSemanticsReference"], "capture", timeout=600)
    raw = catalogs.read_result(stdout, MARKER, 64 * 1024 * 1024)
    if raw["native_version"] != version or len(raw["samples"]) != 108 or len(raw["providers"]) != 12:
        raise ValueError("native tree result identity or requested coverage differs")
    result = {**raw, "jar_sha256": native.JAR_SHA256, "probe_sha256": hashlib.sha256(
        b"".join((output / "sources" / name).read_bytes() for name in SOURCES)).hexdigest()}
    native.write_json(output / FIXTURE, result)
    if repeat is not None:
        if result != previous:
            raise ValueError("independent native tree repeat differs")
        old_environment = json.loads((repeat / "environment.json").read_bytes())
        for key in ("version", "jar_sha256", "libraries", "java_executable_sha256", "javac_executable_sha256",
                    "python_executable_sha256", "runtime_environment"):
            if evidence[key] != old_environment[key]:
                raise ValueError(f"independent tree environment differs: {key}")
    for name, expected in sources.items():
        if native.sha(native.ROOT / name) != expected:
            raise ValueError(f"tree source changed during capture: {name}")
    native.environment()
    manifest = {"schema": "bcore.native-tree-capture.v1", "jar_sha256": native.JAR_SHA256,
                "source_sha256": sources, "historical_source_sha256": historical,
                "sample_count": 108, "provider_count": 12, "independent_repeat": repeat is not None,
                "repeat_manifest_sha256": pin,
                "files_sha256": {path.relative_to(output).as_posix(): native.sha(path)
                                 for path in sorted(output.rglob("*")) if path.is_file()}}
    native.write_json(output / "manifest.json", manifest)
    verified(output, native.sha(output / "manifest.json"))
    print(json.dumps({"output": str(output), "status": "pass", "sample_count": 108,
                      "provider_count": 12, "manifest_sha256": native.sha(output / "manifest.json")}, indent=2))


def promote(source: Path, pin: str) -> None:
    native.environment()
    manifest, _ = verified(source, pin)
    if not manifest["independent_repeat"]:
        raise ValueError("tree fixture promotion requires an independent native repeat")
    destination = native.ROOT / "crates/bcore-worldgen/data" / FIXTURE
    expected = manifest["files_sha256"][FIXTURE]
    if destination.exists():
        if native.sha(destination) != expected:
            raise ValueError("different target tree fixture already exists")
    else:
        with destination.open("xb") as stream:
            stream.write((source / FIXTURE).read_bytes())
    if native.sha(destination) != expected:
        raise ValueError("tree promotion readback differs")
    print(json.dumps({"status": "pass", "destination": str(destination), "sha256": expected,
                      "capture_manifest_sha256": pin}, indent=2))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("capture", "promote"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--manifest-sha256")
    args = parser.parse_args()
    if args.mode == "capture":
        if args.output is None or bool(args.source) != bool(args.manifest_sha256):
            parser.error("capture needs --output; a repeat needs both --source and --manifest-sha256")
        output = args.output.resolve()
        if output.exists() or not output.parent.is_dir() or not output.is_relative_to(native.ROOT / "target"):
            parser.error("output must be absent with an existing parent under target/")
        capture(output, args.source.resolve() if args.source else None, args.manifest_sha256)
    else:
        if args.source is None or args.manifest_sha256 is None or args.output is not None:
            parser.error("promote needs --source and --manifest-sha256")
        promote(args.source.resolve(), args.manifest_sha256)


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    main()
