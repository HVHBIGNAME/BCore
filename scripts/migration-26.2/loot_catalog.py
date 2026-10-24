"""Capture genuine 26.2 loot catalogs or native fills on the main session's queue."""
from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import zipfile

import catalogs
import native
import loot_catalog_data as data
import loot_catalog_fixture as fixture

SOURCES = (*data.HISTORICAL_JAVA, "scripts/migration-26.2/LootCatalogAgent.java",
           "scripts/migration-26.2/LootCatalogReference.java")
HELPERS = ("scripts/migration-26.2/loot_catalog.py", "scripts/migration-26.2/loot_catalog_data.py",
           "scripts/migration-26.2/loot_catalog_fixture.py", "scripts/migration-26.2/catalogs.py",
           "scripts/migration-26.2/native.py")
ASM = native.ROOT / "target/full-lifecycle-deps/asm-9.8.jar"
REGISTRY = native.ROOT / "crates/bcore-registry/data/registries_26_2.json"
REGISTRY_PROVENANCE = REGISTRY.with_name("provenance_26_2.json")
MARKER = "LOOT_CATALOG_REFERENCE="


def freeze_sources(output: Path, mode: str, registry: Path) -> dict[str, str]:
    paths = (*SOURCES, *HELPERS, *(fixture.VERIFIER_SOURCES if mode == "fill" else ()))
    hashes = {name: catalogs.snapshot(native.ROOT / name, output / "sources" / name) for name in paths}
    pins = {**data.HISTORICAL_JAVA, **(fixture.VERIFIER_SOURCES if mode == "fill" else {})}
    for name, expected in pins.items():
        data.require(hashes[name] == expected, f"historical source changed: {name}")
    catalogs.snapshot(registry, output / "sources/registry_26_2.json")
    catalogs.snapshot(REGISTRY_PROVENANCE, output / "sources/registry-provenance_26_2.json")
    return hashes


def table_inventory() -> dict:
    rows, total = [], 0
    with zipfile.ZipFile(native.JAR) as archive:
        for entry in sorted(archive.infolist(), key=lambda value: value.filename):
            prefix = "data/minecraft/loot_table/"
            if not entry.filename.startswith(prefix) or not entry.filename.endswith(".json"):
                continue
            total += entry.file_size
            data.require(entry.file_size <= catalogs.NBT_LIMIT and total <= catalogs.RESOURCE_LIMIT and len(rows) < 8192,
                         "official JAR loot resources exceed the inventory bound")
            raw = archive.read(entry)
            key = "minecraft:" + entry.filename.removeprefix(prefix).removesuffix(".json")
            data.resource(key, "official JAR loot tables")
            data.require(isinstance(data.decode(raw), dict), "official loot resource is not an object")
            rows.append({"key": key, "jar_member": entry.filename, "sha256": data.digest(raw), "bytes": len(raw)})
    keys = sorted(row["key"] for row in rows)
    data.require(keys and len(keys) == len(set(keys)), "empty or duplicate official loot table inventory")
    return {"schema": "bcore.migration.loot-table-inventory.v1", "minecraft": "26.2", "protocol": 776,
            "jar_sha256": native.JAR_SHA256, "tables": rows, "all_jar_keys": keys,
            "chest_and_trial_keys": [key for key in keys if key.startswith("minecraft:chests/") or "trial_chambers/" in key]}


def compile_probe(output: Path, javac: str, asm: Path, environment: dict, timeout: int, log_limit: int) -> Path:
    build = output / "classes"
    build.mkdir()
    command = [javac, f"-J-Djava.io.tmpdir={output / 'tmp'}", "--release", "21", "-encoding", "UTF-8",
               "-implicit:none", "-cp", str(asm), "-d", str(build),
               *[str(output / "sources" / name) for name in SOURCES]]
    catalogs.bounded_execute(command, output, "compile", timeout=min(timeout, 120), log_limit=log_limit, environment=environment)
    return build


def server_command(output: Path, build: Path, asm: Path, libraries: list[Path], spec: dict) -> list[str]:
    agent = output / "loot-catalog-agent.jar"
    with zipfile.ZipFile(agent, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: LootCatalogAgent\n\n")
        for path in sorted(build.glob("*.class")):
            archive.write(path, path.name)
    properties = "\n".join(("level-name=loot-catalog-world", f"level-seed={spec['world_seed']}", "level-type=minecraft:normal",
                            "generate-structures=true", "server-ip=127.0.0.1", "server-port=0", "online-mode=false",
                            "enable-rcon=false", "enable-query=false", "management-server-enabled=false", "sync-chunk-writes=false",
                            "max-tick-time=-1", "view-distance=2", "simulation-distance=2", "pause-when-empty-seconds=0",
                            "enforce-secure-profile=false", ""))
    for name, text in (("server.properties", properties), ("eula.txt", "eula=true\n")):
        with (output / name).open("x", encoding="utf-8", newline="\n") as stream:
            stream.write(text)
    return [str(native.JAVA), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
            f"-Djava.io.tmpdir={output / 'tmp'}", f"-javaagent:{agent}={output / 'config.json'}", "-cp",
            os.pathsep.join(map(str, [agent, asm, native.JAR, *libraries])), "net.minecraft.server.Main", "--nogui"]


def repeat_preflight(repeat: Path | None, pin: str | None, inputs: dict, environment: dict) -> None:
    if repeat is None:
        return
    manifest, _ = data.verified_capture(repeat, pin)
    previous = data.load(repeat / "capture-inputs.json")
    for key in ("mode", "source_sha256", "historical_source_sha256", "input_spec_sha256", "registry_sha256",
                "registry_provenance_sha256", "asm_sha256"):
        data.require(previous[key] == inputs[key], f"independent repeat input authority differs: {key}")
    old_environment = data.load(repeat / "environment.json")
    for key in ("version", "jar_sha256", "preparation_sha256", "java_executable_sha256", "javac_executable_sha256",
                "python_executable_sha256", "libraries", "runtime_environment"):
        data.require(old_environment[key] == environment[key], f"independent repeat environment differs: {key}")
    data.require(inputs["input_authority"] == previous["input_authority"], "independent repeat input provenance differs")
    data.require(manifest["mode"] == inputs["mode"], "independent repeat mode differs")


def check_repeat(output: Path, repeat: Path, counts: dict) -> dict:
    names = [*data.RUNTIME_FILES, "native-result.json"]
    # The metadata carries execution counts, which are stable for equal explicit inputs.
    equal = {}
    for name in names:
        current, previous = data.load(output / name), data.load(repeat / name)
        data.require(current == previous, f"independent native loot repeat differs: {name}")
        equal[name] = native.sha(output / name)
    classes = {path.name: native.sha(path) for path in sorted((output / "classes").glob("*.class"))}
    previous_classes = {path.name: native.sha(path) for path in sorted((repeat / "classes").glob("*.class"))}
    data.require(classes == previous_classes, "independent repeat compiled classes differ")
    return {"status": "pass", "equal_exports_sha256": equal, "counts": counts}


def run_capture(args, output: Path, version: dict, libraries: list[Path], javac: str, spec: dict, input_authority: dict) -> dict:
    output.mkdir()
    sources = freeze_sources(output, args.mode, args.registry.resolve())
    asm = output / "tools/asm-9.8.jar"
    data.require(catalogs.snapshot(args.asm.resolve(), asm) == catalogs.ASM_SHA256, "ASM source changed while freezing")
    environment = dict(os.environ)
    temporary = output / "tmp"
    log_limit = args.max_log_mib * 1024 * 1024
    evidence = catalogs.capture_environment(output, libraries, version, javac, asm, args.asm.resolve(), log_limit)
    environment.update({key: str(temporary) for key in ("TMP", "TEMP", "TMPDIR")})
    native.write_json(output / "input-spec.json", spec)
    config = {**spec, "output": str(output), "mode": args.mode, "minecraft": "26.2", "protocol": 776,
              "jar_sha256": native.JAR_SHA256}
    native.write_json(output / "config.json", config)
    inputs = {"schema": data.CAPTURE_SCHEMA, "minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
              "mode": args.mode, "source_sha256": sources, "historical_source_sha256": data.HISTORICAL_JAVA,
              "java_source_order": list(SOURCES), "input_authority": input_authority,
              "input_spec_sha256": native.sha(output / "input-spec.json"), "asm_sha256": native.sha(asm),
              "registry_sha256": native.sha(output / "sources/registry_26_2.json"),
              "registry_provenance_sha256": native.sha(output / "sources/registry-provenance_26_2.json"),
              "environment_sha256": native.sha(output / "environment.json"),
              "bootstrap": "MinecraftServer.setInitialSpawn entry", "gameplay_ticks": 0,
              "lookup": "MinecraftServer.reloadableRegistries().lookup().createSerializationContext(JsonOps.INSTANCE)"}
    native.write_json(output / "capture-inputs.json", inputs)
    repeat = args.repeat_of.resolve() if args.repeat_of is not None else None
    repeat_preflight(repeat, args.repeat_manifest_sha256, inputs, evidence)
    build = compile_probe(output, javac, asm, environment, args.timeout, log_limit)
    command = server_command(output, build, asm, libraries, spec)
    stdout = catalogs.bounded_execute(command, output, "capture", timeout=args.timeout, log_limit=log_limit, environment=environment)
    raw = catalogs.read_result(stdout, MARKER, log_limit)
    data.identity(raw, "native loot completion marker")
    data.require(raw["world_seed_i64"] == spec["world_seed"], "native server used a different world seed")
    native.write_json(output / "native-result.json", raw)
    native.write_json(output / "table-inventory.json", table_inventory())
    payloads = {name: data.load(output / name) for name in data.RUNTIME_FILES}
    data.require(raw == payloads["loot-catalog.json"], "native loot marker and catalog bytes disagree")
    counts = data.validate_catalog(payloads, data.load(output / "sources/registry_26_2.json"))
    if args.mode == "catalog":
        data.require(raw["case_count"] == raw["event_count"] == 0 and (output / "trace.jsonl").stat().st_size == 0,
                     "catalog-only capture executed fill observations")
        verification = {"status": "pass", "case_count": 0, "native_fills": 0}
    else:
        data.require((output / "trace.jsonl").stat().st_size <= data.TRACE_LIMIT, "native loot trace exceeds its byte bound")
        verification = fixture.verify_fill(output, spec, repeat)
    repeated = None if repeat is None else check_repeat(output, repeat, counts)
    if repeat is not None:
        data.verified_capture(repeat, args.repeat_manifest_sha256)
        native.write_json(output / "independent-repeat.json", repeated)
    final_version, final_libraries = native.environment()
    data.require(final_version == version and final_libraries == libraries, "target environment changed during capture")
    for name, expected in sources.items():
        data.require(native.sha(native.ROOT / name) == expected and native.sha(output / "sources" / name) == expected,
                     f"source changed during capture: {name}")
    for path, key in ((native.JAVA, "java_executable_sha256"), (Path(javac), "javac_executable_sha256"),
                      (Path(sys.executable), "python_executable_sha256"), (native.PREPARATION, "preparation_sha256")):
        data.require(native.sha(path) == evidence[key], f"capture environment changed: {path}")
    data.require(native.sha(args.asm.resolve()) == native.sha(asm) == catalogs.ASM_SHA256, "ASM changed during capture")
    data.require(native.sha(args.registry.resolve()) == inputs["registry_sha256"]
                 and native.sha(REGISTRY_PROVENANCE) == inputs["registry_provenance_sha256"],
                 "independent registry authority changed during capture")
    if args.cases is not None:
        data.require(native.sha(args.cases.resolve()) == input_authority["source_sha256"], "case source changed during capture")
    elif args.mode == "fill":
        data.require(native.sha(fixture.HISTORICAL_FIXTURE) == fixture.HISTORICAL_SHA256, "historical fixture changed during capture")
    files = {}
    for path in sorted(output.rglob("*")):
        relative = path.relative_to(output)
        if path.is_file() and relative.parts[0] not in ("loot-catalog-world", "tmp", "logs", "crash-reports"):
            files[relative.as_posix()] = native.sha(path)
    result = {"schema": data.CAPTURE_SCHEMA, "minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
              "mode": args.mode, "status": "pass", "counts": counts, "case_count": verification["case_count"],
              "native_fills": verification["native_fills"], "independent_repeat": repeat is not None,
              "repeat_manifest_sha256": args.repeat_manifest_sha256,
              "files_sha256": files}
    native.write_json(output / "manifest.json", result)
    data.verified_capture(output, native.sha(output / "manifest.json"))
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("catalog", "fill", "list"))
    parser.add_argument("--output", type=Path, help="absent directory with an existing parent under target/")
    parser.add_argument("--cases", type=Path, help="schema-1 inputs; fill defaults to the 137 historical inputs only")
    parser.add_argument("--repeat-of", type=Path)
    parser.add_argument("--repeat-manifest-sha256")
    parser.add_argument("--registry", type=Path, default=REGISTRY)
    parser.add_argument("--asm", type=Path, default=ASM)
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--max-log-mib", type=int, default=64)
    args = parser.parse_args()
    if args.mode == "list":
        print(json.dumps({"modes": {"catalog": {"cases": 0}, "fill": {"default_input_cases": 137, "maximum_cases": 256}},
                          "runtime_files": data.RUNTIME_FILES, "java_sources": SOURCES, "jar_sha256": native.JAR_SHA256,
                          "asm": str(ASM), "asm_sha256": catalogs.ASM_SHA256,
                          "marker": MARKER, "bootstrap": "MinecraftServer.setInitialSpawn", "gameplay_ticks": 0}, indent=2))
        return
    if args.output is None or args.timeout <= 0 or not 1 <= args.max_log_mib <= 256:
        parser.error("provide --output, a positive timeout, and a 1..256 MiB log bound")
    if bool(args.repeat_of) != bool(args.repeat_manifest_sha256):
        parser.error("--repeat-of requires --repeat-manifest-sha256, and vice versa")
    output = args.output.resolve()
    if output.exists() or not output.parent.is_dir() or not output.is_relative_to(native.ROOT / "target"):
        parser.error("output must be absent with an existing parent under target/")
    if args.cases is not None and args.mode != "fill":
        parser.error("--cases applies only to fill")
    if os.environ.get("CARGO_BUILD_JOBS") != "1":
        parser.error("launch through scripts/run_generation_check.py on main's queue")
    spec, input_authority = fixture.input_spec(args.mode, args.cases)
    version, libraries = native.environment()
    data.require(len(libraries) == 39, "expected exactly 39 authenticated official 26.2 libraries")
    data.require(native.sha(args.asm.resolve()) == catalogs.ASM_SHA256, "ASM 9.8 SHA-256 differs")
    registry = data.load(args.registry.resolve())
    data.identity(registry, "independent native registry")
    registry_provenance = data.load(REGISTRY_PROVENANCE)
    data.identity(registry_provenance, "native registry provenance")
    data.require(registry_provenance["generated_sha256"]["registries_26_2.json"] == native.sha(args.registry.resolve()),
                 "independent registry publication digest differs")
    for name, expected in {**data.HISTORICAL_JAVA, **(fixture.VERIFIER_SOURCES if args.mode == "fill" else {})}.items():
        data.require(native.sha(native.ROOT / name) == expected, f"historical source changed: {name}")
    javac = shutil.which("javac")
    data.require(javac is not None, "javac 21+ is missing")
    try:
        result = run_capture(args, output, version, libraries, javac, spec, input_authority)
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        if output.is_dir():
            native.write_json(output / "failure.json", {"status": "fail", "error": f"{type(error).__name__}: {error}"})
        raise
    print(json.dumps({"output": str(output), "status": result["status"], "mode": args.mode, "counts": result["counts"],
                      "cases": result["case_count"], "native_fills": result["native_fills"],
                      "manifest_sha256": native.sha(output / "manifest.json")}, indent=2), flush=True)


if __name__ == "__main__":
    main()
