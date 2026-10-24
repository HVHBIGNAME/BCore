"""Capture and independently repeat native loot location checks on the shared queue."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import sys
import zipfile

sys.dont_write_bytecode = True

import catalogs
import loot_catalog
import loot_catalog_data as data
import native

SOURCES = (*data.HISTORICAL_JAVA, "scripts/migration-26.2/LootCatalogReference.java",
           "scripts/migration-26.2/LootLocationAgent.java", "scripts/migration-26.2/LootLocationReference.java")
HELPERS = ("scripts/migration-26.2/loot_location.py", *loot_catalog.HELPERS)
FIXTURE = "loot_location_26_2.json"


def verified(source: Path, pin: str) -> tuple[dict, dict]:
    data.require(native.sha(source / "manifest.json") == pin, "loot location manifest changed")
    manifest = data.load(source / "manifest.json")
    data.identity(manifest, "loot location manifest")
    for name, expected in manifest["files_sha256"].items():
        path = source / name
        data.require(path.resolve().is_relative_to(source) and native.sha(path) == expected,
                     f"loot location capture changed: {name}")
    result = data.load(source / FIXTURE)
    data.identity(result, "loot location observations")
    return manifest, result


def capture(output: Path, repeat: Path | None, pin: str | None) -> None:
    version, libraries = native.environment()
    javac = shutil.which("javac")
    data.require(javac is not None and len(libraries) == 39, "javac and authenticated native libraries are required")
    if repeat is not None:
        previous_manifest, previous = verified(repeat, pin)
    output.mkdir()
    sources = {name: catalogs.snapshot(native.ROOT / name, output / "sources" / name)
               for name in (*SOURCES, *HELPERS)}
    for name, expected in data.HISTORICAL_JAVA.items():
        data.require(sources[name] == expected, f"historical probe source changed: {name}")
    if repeat is not None:
        data.require(sources == previous_manifest["source_sha256"], "loot location repeat sources differ")
    asm = output / "tools/asm-9.8.jar"
    data.require(catalogs.snapshot(loot_catalog.ASM, asm) == catalogs.ASM_SHA256, "ASM digest differs")
    evidence = catalogs.capture_environment(output, libraries, version, javac, asm, loot_catalog.ASM, data.JSON_LIMIT)
    environment = dict(os.environ)
    environment.update({key: str(output / "tmp") for key in ("TMP", "TEMP", "TMPDIR")})
    build = output / "classes"
    build.mkdir()
    catalogs.bounded_execute([javac, "--release", "21", "-encoding", "UTF-8", "-implicit:none", "-cp", str(asm),
                              "-d", str(build), *[str(output / "sources" / name) for name in SOURCES]],
                             output, "compile", timeout=120, log_limit=data.JSON_LIMIT, environment=environment)
    agent = output / "loot-location-agent.jar"
    with zipfile.ZipFile(agent, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: LootLocationAgent\n\n")
        for path in sorted(build.glob("*.class")):
            archive.write(path, path.name)
    spec = {"mode": "catalog", "output": str(output), "world_seed": "846692123413862008"}
    native.write_json(output / "config.json", spec)
    for name, text in (("eula.txt", "eula=true\n"), ("server.properties", "\n".join((
            "level-name=loot-location-world", f"level-seed={spec['world_seed']}", "generate-structures=true",
            "server-ip=127.0.0.1", "server-port=0", "online-mode=false", "enable-rcon=false",
            "enable-query=false", "management-server-enabled=false", "sync-chunk-writes=false",
            "max-tick-time=-1", "view-distance=2", "simulation-distance=2", "")))):
        with (output / name).open("x", encoding="utf-8", newline="\n") as stream:
            stream.write(text)
    command = [str(native.JAVA), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
               f"-Djava.io.tmpdir={output / 'tmp'}", f"-javaagent:{agent}={output / 'config.json'}", "-cp",
               os.pathsep.join(map(str, [agent, asm, native.JAR, *libraries])), "net.minecraft.server.Main", "--nogui"]
    stdout = catalogs.bounded_execute(command, output, "capture", timeout=900,
                                     log_limit=data.JSON_LIMIT, environment=environment)
    result = catalogs.read_result(stdout, "LOOT_LOCATION_REFERENCE=", data.JSON_LIMIT)
    data.identity(result, "native loot location result")
    data.require(len(result["samples"]) == 20 and result["gameplay_ticks"] == 0, "loot location coverage differs")
    result["source_sha256"] = {name: sources[name] for name in SOURCES}
    native.write_json(output / FIXTURE, result)
    if repeat is not None:
        data.require(result == previous, "independent native location observations differ")
        old_environment = data.load(repeat / "environment.json")
        for key in ("version", "jar_sha256", "libraries", "java_executable_sha256", "javac_executable_sha256",
                    "python_executable_sha256", "runtime_environment"):
            data.require(evidence[key] == old_environment[key], f"loot location repeat environment differs: {key}")
    for name, expected in sources.items():
        data.require(native.sha(native.ROOT / name) == expected, f"loot location source changed during capture: {name}")
    native.environment()
    manifest = {"schema": "bcore.native-loot-location-capture.v1", "minecraft": "26.2", "protocol": 776,
                "jar_sha256": native.JAR_SHA256, "source_sha256": sources,
                "independent_repeat": repeat is not None, "repeat_manifest_sha256": pin,
                "files_sha256": {path.relative_to(output).as_posix(): native.sha(path) for path in sorted(output.rglob("*"))
                                 if path.is_file() and path.relative_to(output).parts[0] not in
                                 ("loot-location-world", "logs", "tmp", "crash-reports")}}
    native.write_json(output / "manifest.json", manifest)
    verified(output, native.sha(output / "manifest.json"))
    print(json.dumps({"status": "pass", "output": str(output), "sample_count": 20,
                      "manifest_sha256": native.sha(output / "manifest.json")}, indent=2))


def promote(source: Path, pin: str) -> None:
    native.environment()
    manifest, _ = verified(source, pin)
    data.require(manifest["independent_repeat"], "loot location promotion requires an independent repeat")
    destination = native.ROOT / "crates/bcore-worldgen/tests/data/loot_26_2" / FIXTURE
    expected = manifest["files_sha256"][FIXTURE]
    if destination.exists():
        data.require(native.sha(destination) == expected, "different loot location fixture already exists")
    else:
        with destination.open("xb") as stream:
            stream.write((source / FIXTURE).read_bytes())
    data.require(native.sha(destination) == expected, "loot location promotion readback differs")
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
    main()
