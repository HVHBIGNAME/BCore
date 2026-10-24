"""Capture authenticated 26.2 math/noise/router values with the preserved native probe."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import sys

import catalogs
import native

SOURCES = ("scripts/TreeReference.java", "scripts/NativeWorldgenRegistries.java",
           "scripts/NumericReference.java", "scripts/migration-26.2/IntervalSelectReference.java")
HELPERS = ("scripts/numeric_cases.py", "scripts/migration-26.2/numeric.py",
           "scripts/migration-26.2/native.py", "scripts/migration-26.2/catalogs.py")
FIXTURE = "numeric_26_2.json"
INTERVAL_FIXTURE = "interval_select_26_2.json"
ADAPTED_PROBE = "derived/NumericReference.java"


def adapt_probe(source: bytes) -> bytes:
    old = b'field("world.level.levelgen.DensityFunction", "HOLDER_HELPER_CODEC")'
    new = b'field("world.level.levelgen.DensityFunction", "CODEC")'
    if source.count(old) != 1:
        raise ValueError("preserved numeric probe codec seam differs")
    marker = b'field("world.level.levelgen.DensityFunctions$BeardifierMarker", "INSTANCE")'
    if source.count(marker) != 1:
        raise ValueError("preserved numeric probe beardifier seam differs")
    return source.replace(old, new).replace(marker, b'IntervalSelectReference.beardifier()')


def requests() -> dict:
    import numeric_cases

    result = numeric_cases.requests(native.JAR)
    result["removed_codecs"] = [case for case in result["cases"]
                                if isinstance(case.get("function"), dict)
                                and case["function"].get("type") == "minecraft:weird_scaled_sampler"]
    removed_ids = {case["id"] for case in result["removed_codecs"]}
    result["cases"] = [case for case in result["cases"] if case["id"] not in removed_ids]
    boundaries = [-1.0, -0.0, 0.0, 1.0]
    values = [-2.0, 2.0]
    for value in boundaries:
        values.extend((math.nextafter(value, -math.inf), value,
                       math.nextafter(value, math.inf)))
    for index, value in enumerate(values):
        result["cases"].append({
            "id": f"interval/boundary/{index}", "op": "density", "seed": 17,
            "points": "blocks", "function": {
                "type": "minecraft:interval_select", "input": value,
                "thresholds": [-1.0, -0.0, 0.0, 1.0],
                "functions": [-0.0, -2.0, 3.0, 4.0, 5.0],
            },
        })
    result["cases"].append({
        "id": "interval/gradient", "op": "density", "seed": 17,
        "points": "blocks", "function": {
            "type": "minecraft:interval_select", "input": {
                "type": "minecraft:y_clamped_gradient", "from_y": -64, "to_y": 319,
                "from_value": -64.0, "to_value": 319.0,
            },
            "thresholds": [-64.0, 0.0, 63.0, 319.0],
            "functions": [-0.0, -2.0, 3.0, 4.0, 5.0],
        },
    })
    definition = {"type": "minecraft:interval_select", "input": 0.0,
                  "thresholds": [-1.0, -0.0, 0.0, 1.0],
                  "functions": [-0.0, -2.0, 3.0, 4.0, 5.0]}
    result["interval_codecs"] = []
    for name, edits in (
        ("valid", {}), ("duplicate", {"thresholds": [0.0, 0.0], "functions": [1.0, 2.0, 3.0]}),
        ("signed_zero_reverse", {"thresholds": [0.0, -0.0], "functions": [1.0, 2.0, 3.0]}),
        ("unsorted", {"thresholds": [1.0, -1.0], "functions": [1.0, 2.0, 3.0]}),
        ("empty", {"thresholds": [], "functions": []}),
        ("singleton", {"thresholds": [], "functions": [1.0]}),
        ("too_few", {"functions": [1.0, 2.0]}),
        ("too_many", {"functions": [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]}),
        ("threshold_string", {"thresholds": ["0"], "functions": [1.0, 2.0]}),
        ("threshold_bool", {"thresholds": [True], "functions": [1.0, 2.0]}),
        ("lower_limit", {"thresholds": [-1000000.0], "functions": [1.0, 2.0]}),
        ("upper_limit", {"thresholds": [1000000.0], "functions": [1.0, 2.0]}),
        ("below_limit", {"thresholds": [-1000000.0000000001], "functions": [1.0, 2.0]}),
        ("above_limit", {"thresholds": [1000000.0000000001], "functions": [1.0, 2.0]}),
        ("invalid_child", {"functions": [1.0, 2.0, 3.0, 4.0, {"type": "minecraft:missing"}]}),
    ):
        result["interval_codecs"].append({"id": name, "function": {**definition, **edits}})
    for key in ("input", "thresholds", "functions"):
        result["interval_codecs"].append({
            "id": f"missing_{key}", "function": {name: value for name, value in definition.items() if name != key},
        })
    result["interval_runtime"] = {
        "thresholds": definition["thresholds"], "functions": definition["functions"],
        "inputs": ["fff0000000000000", "c000000000000000", "bff0000000000000",
                   "8000000000000000", "0000000000000000", "3ff0000000000000",
                   "4000000000000000", "7ff0000000000000", "7ff8000000000011"],
    }
    return result


def verified(source: Path, pin: str) -> tuple[dict, dict]:
    if native.sha(source / "manifest.json") != pin:
        raise ValueError("numeric capture manifest digest differs")
    manifest = json.loads((source / "manifest.json").read_bytes())
    if (manifest["minecraft"], manifest["protocol"], manifest["jar_sha256"],
            manifest["status"]) != ("26.2", 776, native.JAR_SHA256, "pass"):
        raise ValueError("numeric capture identity or completion differs")
    for name, expected in manifest["files_sha256"].items():
        path = source / name
        if not path.resolve().is_relative_to(source) or native.sha(path) != expected:
            raise ValueError(f"numeric capture file changed: {name}")
    fixture = json.loads((source / FIXTURE).read_bytes())
    if (fixture["minecraft"], fixture["protocol"], fixture["jar_sha256"]) != (
            "26.2", 776, native.JAR_SHA256):
        raise ValueError("numeric fixture belongs to a different target")
    return manifest, fixture


def capture(output: Path, repeat: Path | None, pin: str | None) -> None:
    version, libraries = native.environment()
    javac = shutil.which("javac")
    if javac is None or len(libraries) != 39:
        raise ValueError("javac and all 39 authenticated target libraries are required")
    previous = verified(repeat, pin) if repeat is not None else None
    output.mkdir()
    sources = {name: catalogs.snapshot(native.ROOT / name, output / "sources" / name)
               for name in (*SOURCES, *HELPERS)}
    historical = catalogs.preserved_source_hashes(sources)
    adapted = output / "sources" / ADAPTED_PROBE
    adapted.parent.mkdir()
    with adapted.open("xb") as stream:
        stream.write(adapt_probe((output / "sources/scripts/NumericReference.java").read_bytes()))
    derived = {ADAPTED_PROBE: native.sha(adapted)}
    if previous is not None and sources != previous[0]["source_sha256"]:
        raise ValueError("independent numeric repeat sources differ")
    environment = dict(os.environ)
    environment.update({key: str(output / "tmp") for key in ("TMP", "TEMP", "TMPDIR")})
    evidence = catalogs.capture_environment(output, libraries, version, javac, None, None,
                                           64 * 1024 * 1024)
    sys.path.insert(0, str(native.ROOT / "scripts"))
    inputs = requests()
    native.write_json(output / "requests.json", inputs)

    def execute(command, name, *, timeout):
        return catalogs.bounded_execute(command, output, name, timeout=timeout,
                                        log_limit=64 * 1024 * 1024, environment=environment)

    executed_sources = tuple(ADAPTED_PROBE if name == "scripts/NumericReference.java" else name for name in SOURCES)
    build = catalogs.compile_probe(catalogs.Probe("IntervalSelectReference", executed_sources, FIXTURE),
                                   output, javac, None, execute)
    stdout = execute([str(native.JAVA), "-Xmx1G", "-XX:ActiveProcessorCount=2",
                      f"-Djava.io.tmpdir={output / 'tmp'}", "-cp",
                      os.pathsep.join(map(str, [build, native.JAR, *libraries])),
                      "IntervalSelectReference", str(output / "requests.json")], "capture", timeout=1200)
    result = catalogs.read_result(stdout, "NUMERIC_REFERENCE=", 64 * 1024 * 1024)
    if result["cases"] != inputs["cases"] or result["points"] != inputs["points"]:
        raise ValueError("native numeric probe changed its explicit inputs")
    samples = result["samples"]
    if [sample["id"] for sample in samples] != [case["id"] for case in inputs["cases"]]:
        raise ValueError("native numeric case identity/order is incomplete")
    for case, sample in zip(inputs["cases"], samples, strict=True):
        width = 8 if case["op"] == "lerp" and case["precision"] == "f32" else 16
        if (len(sample["bits"]) != len(inputs["points"][case["points"]])
                or any(len(bits) != width or any(c not in "0123456789abcdef" for c in bits)
                       for bits in sample["bits"])):
            raise ValueError(f"native numeric bits are incomplete: {case['id']}")
    result.update(minecraft="26.2", protocol=776, data_version=4903,
                  jar_sha256=native.JAR_SHA256,
                  probe_sha256=hashlib.sha256(b"".join(
                       (output / "sources" / name).read_bytes() for name in executed_sources)).hexdigest())
    native.write_json(output / FIXTURE, result)
    interval = catalogs.read_result(stdout, "INTERVAL_REFERENCE=", 64 * 1024 * 1024)
    if ([row["id"] for row in interval["codecs"]] != [row["id"] for row in inputs["interval_codecs"]]
            or [row["input_bits"] for row in interval["runtime"]] != inputs["interval_runtime"]["inputs"]
            or [row["id"] for row in interval["removed_codecs"]] != [row["id"] for row in inputs["removed_codecs"]]):
        raise ValueError("native interval-select matrix identity/order is incomplete")
    interval.update(schema="bcore.native-interval-select.v1", minecraft="26.2", protocol=776,
                    data_version=4903, jar_sha256=native.JAR_SHA256,
                    probe_sha256=result["probe_sha256"], inputs={
                        "codecs": inputs["interval_codecs"], "runtime": inputs["interval_runtime"],
                        "removed_codecs": inputs["removed_codecs"],
                    })
    native.write_json(output / INTERVAL_FIXTURE, interval)
    if previous is not None:
        if result != previous[1]:
            raise ValueError("independent native numeric repeat differs")
        if interval != json.loads((repeat / INTERVAL_FIXTURE).read_bytes()):
            raise ValueError("independent native interval-select repeat differs")
        if derived != previous[0]["derived_source_sha256"]:
            raise ValueError("independent numeric probe adaptation differs")
        old_environment = json.loads((repeat / "environment.json").read_bytes())
        for key in ("version", "jar_sha256", "libraries", "java_executable_sha256",
                    "javac_executable_sha256", "python_executable_sha256", "runtime_environment"):
            if evidence[key] != old_environment[key]:
                raise ValueError(f"independent numeric environment differs: {key}")
    for name, expected in sources.items():
        if native.sha(native.ROOT / name) != expected:
            raise ValueError(f"numeric source changed during capture: {name}")
    native.environment()
    manifest = {
        "schema": "bcore.native-numeric-capture.v1", "minecraft": "26.2", "protocol": 776,
        "jar_sha256": native.JAR_SHA256, "status": "pass", "source_sha256": sources,
        "derived_source_sha256": derived,
        "adaptations": [{"source": "scripts/NumericReference.java", "derived": ADAPTED_PROBE,
                         "seams": ["DensityFunction.HOLDER_HELPER_CODEC -> DensityFunction.CODEC",
                                   "BeardifierMarker.INSTANCE -> accessible target field lookup"]}],
        "historical_source_sha256": historical, "case_count": len(samples),
        "value_count": sum(len(sample["bits"]) for sample in samples),
        "independent_repeat": previous is not None, "repeat_manifest_sha256": pin,
        "files_sha256": {path.relative_to(output).as_posix(): native.sha(path)
                         for path in sorted(output.rglob("*")) if path.is_file()},
    }
    native.write_json(output / "manifest.json", manifest)
    verified(output, native.sha(output / "manifest.json"))
    print(json.dumps({"output": str(output), "status": "pass", "cases": len(samples),
                      "values": manifest["value_count"],
                      "manifest_sha256": native.sha(output / "manifest.json")}, indent=2))


def promote(source: Path, pin: str) -> None:
    native.environment()
    manifest, _ = verified(source, pin)
    if not manifest["independent_repeat"]:
        raise ValueError("numeric fixture promotion requires an independent native repeat")
    destinations = {}
    for name in (FIXTURE, INTERVAL_FIXTURE):
        destination = native.ROOT / "crates/bcore-worldgen/data" / name
        expected = manifest["files_sha256"][name]
        if destination.exists() and native.sha(destination) != expected:
            raise ValueError(f"different target numeric fixture already exists: {name}")
        destinations[destination] = expected
    for destination, expected in destinations.items():
        if not destination.exists():
            with destination.open("xb") as stream:
                stream.write((source / destination.name).read_bytes())
        if native.sha(destination) != expected:
            raise ValueError("numeric promotion readback differs")
    print(json.dumps({"status": "pass", "destinations": {str(path): sha for path, sha in destinations.items()},
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
    sys.dont_write_bytecode = True
    main()
