"""Capture, repeat and promote native target speleothem operations and codec inputs."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil

import catalogs
import native

ORIGINAL = (*catalogs.COMMON, "scripts/CaveFeatureReference.java")
SOURCES = (*ORIGINAL, "scripts/migration-26.2/SpeleothemReference.java")
HELPERS = ("scripts/migration-26.2/speleothem.py", "scripts/migration-26.2/native.py",
           "scripts/migration-26.2/catalogs.py")
FIXTURE = "speleothem_26_2.json"


def verified(source: Path, pin: str) -> tuple[dict, dict]:
    if native.sha(source / "manifest.json") != pin:
        raise ValueError("speleothem manifest changed")
    manifest = json.loads((source / "manifest.json").read_bytes())
    if manifest["jar_sha256"] != native.JAR_SHA256:
        raise ValueError("speleothem capture target differs")
    for name, expected in manifest["files_sha256"].items():
        path = source / name
        if not path.resolve().is_relative_to(source) or native.sha(path) != expected:
            raise ValueError(f"speleothem evidence changed: {name}")
    result = json.loads((source / FIXTURE).read_bytes())
    if (result["minecraft"], result["protocol"], result["jar_sha256"]) != ("26.2", 776, native.JAR_SHA256):
        raise ValueError("speleothem fixture identity differs")
    if len(result["samples"]) != 102 or len(result["admission"]) != 18 or len(result["configs"]) != 4:
        raise ValueError("speleothem fixture coverage differs")
    return manifest, result


def capture(output: Path, repeat: Path | None, pin: str | None) -> None:
    version, libraries = native.environment()
    javac = shutil.which("javac")
    if javac is None or len(libraries) != 39:
        raise ValueError("javac and all 39 authenticated target libraries are required")
    if repeat is not None:
        previous_manifest, previous = verified(repeat, pin)
    historical = json.loads((native.ROOT / "crates/bcore-worldgen/data/cave_features_26_1.json").read_bytes())
    original_pin = hashlib.sha256(b"".join((native.ROOT / name).read_bytes() for name in ORIGINAL)).hexdigest()
    if historical["probe_sha256"] != original_pin:
        raise ValueError("preserved historical cave probe inputs changed")
    output.mkdir()
    sources = {name: catalogs.snapshot(native.ROOT / name, output / "sources" / name)
               for name in (*SOURCES, *HELPERS)}
    if repeat is not None and sources != previous_manifest["source_sha256"]:
        raise ValueError("speleothem repeat sources differ")
    evidence = catalogs.capture_environment(output, libraries, version, javac, None, None, 64 * 1024 * 1024)
    environment = dict(os.environ)
    environment.update({key: str(output / "tmp") for key in ("TMP", "TEMP", "TMPDIR")})

    def execute(command, name, *, timeout):
        return catalogs.bounded_execute(command, output, name, timeout=timeout,
                                        log_limit=64 * 1024 * 1024, environment=environment)

    build = catalogs.compile_probe(catalogs.Probe("SpeleothemReference", SOURCES, FIXTURE),
                                   output, javac, None, execute)
    stdout = execute([str(native.JAVA), "-Xmx1G", "-XX:ActiveProcessorCount=2",
                      f"-Djava.io.tmpdir={output / 'tmp'}", "-cp",
                      os.pathsep.join(map(str, [build, native.JAR, *libraries])), "SpeleothemReference"],
                     "capture", timeout=600)
    raw = catalogs.read_result(stdout, "SPELEOTHEM_REFERENCE=", 64 * 1024 * 1024)
    if raw["native_version"] != version:
        raise ValueError("native speleothem version differs")
    result = {**raw, "minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
              "probe_sha256": hashlib.sha256(b"".join((output / "sources" / name).read_bytes()
                                                        for name in SOURCES)).hexdigest(),
              "historical_probe_sha256": original_pin}
    native.write_json(output / FIXTURE, result)
    if repeat is not None:
        if result != previous:
            raise ValueError("independent speleothem repeat differs")
        old_environment = json.loads((repeat / "environment.json").read_bytes())
        for key in ("version", "jar_sha256", "libraries", "java_executable_sha256",
                    "javac_executable_sha256", "python_executable_sha256", "runtime_environment"):
            if evidence[key] != old_environment[key]:
                raise ValueError(f"speleothem repeat environment differs: {key}")
    for name, expected in sources.items():
        if native.sha(native.ROOT / name) != expected:
            raise ValueError(f"speleothem input changed during capture: {name}")
    native.environment()
    native.write_json(output / "manifest.json", {
        "schema": "bcore.native-speleothem-capture.v1", "jar_sha256": native.JAR_SHA256,
        "source_sha256": sources, "historical_probe_sha256": original_pin,
        "sample_count": 102, "admission_count": 18, "independent_repeat": repeat is not None,
        "repeat_manifest_sha256": pin,
        "files_sha256": {path.relative_to(output).as_posix(): native.sha(path)
                         for path in sorted(output.rglob("*")) if path.is_file()},
    })
    verified(output, native.sha(output / "manifest.json"))
    print(json.dumps({"status": "pass", "output": str(output), "sample_count": 102,
                      "admission_count": 18, "manifest_sha256": native.sha(output / "manifest.json")}, indent=2))


def promote(source: Path, pin: str) -> None:
    native.environment()
    manifest, _ = verified(source, pin)
    if not manifest["independent_repeat"]:
        raise ValueError("speleothem promotion requires an independent native repeat")
    destination = native.ROOT / "crates/bcore-worldgen/data" / FIXTURE
    expected = manifest["files_sha256"][FIXTURE]
    if destination.exists():
        if native.sha(destination) != expected:
            raise ValueError("different target speleothem fixture already exists")
    else:
        with destination.open("xb") as stream:
            stream.write((source / FIXTURE).read_bytes())
    if native.sha(destination) != expected:
        raise ValueError("speleothem publication readback differs")
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
            parser.error("capture needs --output; repeat needs --source and --manifest-sha256")
        output = args.output.resolve()
        if output.exists() or not output.parent.is_dir() or not output.is_relative_to(native.ROOT / "target"):
            parser.error("output must be absent with an existing parent under target/")
        capture(output, args.source.resolve() if args.source else None, args.manifest_sha256)
    else:
        if args.source is None or args.manifest_sha256 is None or args.output is not None:
            parser.error("promote needs --source and --manifest-sha256")
        promote(args.source.resolve(), args.manifest_sha256)


if __name__ == "__main__":
    main()
