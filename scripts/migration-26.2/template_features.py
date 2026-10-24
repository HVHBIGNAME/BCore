"""Capture and repeat native sulfur-template features and block-entity codecs."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import zipfile

import catalogs
import native

SOURCES = (*catalogs.JIGSAW, "scripts/migration-26.2/TemplateFeatureAgent.java",
           "scripts/migration-26.2/TemplateFeatureReference.java")
HELPERS = ("scripts/migration-26.2/template_features.py", "scripts/migration-26.2/native.py",
           "scripts/migration-26.2/catalogs.py", "scripts/jigsaw-reference/capture.py")
FIXTURE = "template_feature_effects_26_2.json"
FULL_FIXTURE = "template_features_26_2_v2.json"
REFERENCE = native.ROOT / "crates/bcore-worldgen/data/template_features_26_2.json"
REFERENCE_SHA256 = "d2cb8b52dfd0882595d5041965a3d074cae1e00afc6534d52c1c28a5034f0875"
LOG_LIMIT = 64 * 1024 * 1024
WORLD = "template-feature-world"
SEED = 846692123413862008


def identity(result: dict) -> None:
    if (result["minecraft"], result["protocol"], result["jar_sha256"]) != ("26.2", 776, native.JAR_SHA256):
        raise ValueError("template feature target differs")
    if (len(result["samples"]), len(result["admission"]), len(result["potent_sulfur_loads"])) != (153, 12, 55):
        raise ValueError("template feature coverage differs")
    if result["gameplay_ticks"] != 0 or result["manager_source"] != "initialized-server-structure-manager":
        raise ValueError("template feature native boundary differs")
    for sample in result["samples"]:
        marks = sample.get("explicit_section_marks")
        if not isinstance(marks, list) or any(
                not isinstance(row, list) or len(row) != 3 or any(type(value) is not int for value in row)
                for row in marks):
            raise ValueError("template feature explicit section marks differ")


def reference() -> dict:
    if native.sha(REFERENCE) != REFERENCE_SHA256:
        raise ValueError("original template feature fixture changed")
    return json.loads(REFERENCE.read_bytes())


def project_effects(result: dict, original: dict) -> dict:
    excluded = {"samples", "source_sha256"}
    if ({key: value for key, value in result.items() if key not in excluded}
            != {key: value for key, value in original.items() if key not in excluded}):
        raise ValueError("template feature reference identity or codecs differ")
    if len(result["samples"]) != len(original["samples"]):
        raise ValueError("template feature reference sample count differs")
    samples = []
    for observed, previous in zip(result["samples"], original["samples"], strict=True):
        if {key: value for key, value in observed.items() if key != "explicit_section_marks"} != previous:
            raise ValueError(f"template feature reference sample differs: {previous['name']}")
        samples.append({key: observed[key] for key in ("name", "seed", "terrain", "origin", "explicit_section_marks")})
    return {"schema": "bcore.template-feature-effects.v1", "reference_sha256": REFERENCE_SHA256,
            **{key: result[key] for key in ("minecraft", "protocol", "jar_sha256", "gameplay_ticks", "manager_source", "source_sha256")},
            "samples": samples}


def verified(source: Path, pin: str) -> tuple[dict, dict]:
    if native.sha(source / "manifest.json") != pin:
        raise ValueError("template feature manifest changed")
    manifest = json.loads((source / "manifest.json").read_bytes())
    if manifest["jar_sha256"] != native.JAR_SHA256:
        raise ValueError("template feature manifest target differs")
    for name, expected in manifest["files_sha256"].items():
        path = source / name
        if not path.resolve().is_relative_to(source) or native.sha(path) != expected:
            raise ValueError(f"template feature evidence changed: {name}")
    result = json.loads((source / FULL_FIXTURE).read_bytes())
    identity(result)
    if json.loads((source / FIXTURE).read_bytes()) != project_effects(result, reference()):
        raise ValueError("template feature effects projection differs")
    return manifest, result


def capture(output: Path, repeat: Path | None, pin: str | None) -> None:
    version, libraries = native.environment()
    javac = shutil.which("javac")
    if javac is None or len(libraries) != 39:
        raise ValueError("javac and all 39 authenticated target libraries are required")
    original = reference()
    if repeat is not None:
        previous_manifest, previous = verified(repeat, pin)
    output.mkdir()
    sources = {name: catalogs.snapshot(native.ROOT / name, output / "sources" / name)
               for name in (*SOURCES, *HELPERS)}
    preserved = catalogs.preserved_source_hashes(sources)
    if repeat is not None and sources != previous_manifest["source_sha256"]:
        raise ValueError("template feature repeat sources differ")
    asm = output / "tools/asm-9.8.jar"
    if catalogs.snapshot(catalogs.ASM, asm) != catalogs.ASM_SHA256:
        raise ValueError("ASM digest differs")
    evidence = catalogs.capture_environment(output, libraries, version, javac, asm, catalogs.ASM, LOG_LIMIT)
    environment = dict(os.environ)
    environment.update({key: str(output / "tmp") for key in ("TMP", "TEMP", "TMPDIR")})

    def execute(command, name, *, timeout):
        return catalogs.bounded_execute(command, output, name, timeout=timeout,
                                        log_limit=LOG_LIMIT, environment=environment)

    build = catalogs.compile_probe(catalogs.Probe("TemplateFeatureReference", SOURCES, FIXTURE),
                                   output, javac, asm, execute)
    agent = output / "template-feature-agent.jar"
    with zipfile.ZipFile(agent, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: TemplateFeatureAgent\n\n")
        for path in sorted(build.glob("*.class")):
            archive.write(path, path.name)
    native.write_json(output / "config.json", {"world_seed": str(SEED)})
    properties = "\n".join((f"level-name={WORLD}", f"level-seed={SEED}", "generate-structures=true",
                             "server-ip=127.0.0.1", "server-port=0", "online-mode=false", "enable-rcon=false",
                             "enable-query=false", "management-server-enabled=false", "sync-chunk-writes=false",
                             "max-tick-time=-1", "view-distance=2", "simulation-distance=2", ""))
    for name, text in (("eula.txt", "eula=true\n"), ("server.properties", properties)):
        with (output / name).open("x", encoding="utf-8", newline="\n") as stream:
            stream.write(text)
    stdout = execute([str(native.JAVA), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
                      f"-Djava.io.tmpdir={output / 'tmp'}", f"-javaagent:{agent}={output / 'config.json'}", "-cp",
                      os.pathsep.join(map(str, [agent, asm, native.JAR, *libraries])), "net.minecraft.server.Main", "--nogui"],
                     "capture", timeout=900)
    raw = catalogs.read_result(stdout, "TEMPLATE_FEATURE_REFERENCE=", LOG_LIMIT)
    if raw["native_version"] != version:
        raise ValueError("native template feature version differs")
    native.write_json(output / "native-result.json", raw)
    codec = catalogs.frozen_module(output, "scripts/jigsaw-reference/capture.py")
    result = {**catalogs.decode_nbt(codec, raw), "minecraft": "26.2", "protocol": 776,
              "jar_sha256": native.JAR_SHA256, "source_sha256": {name: sources[name] for name in SOURCES}}
    identity(result)
    native.write_json(output / FULL_FIXTURE, result)
    native.write_json(output / FIXTURE, project_effects(result, original))
    if repeat is not None:
        if result != previous:
            raise ValueError("independent native template feature repeat differs")
        old_environment = json.loads((repeat / "environment.json").read_bytes())
        for key in ("version", "jar_sha256", "libraries", "java_executable_sha256", "javac_executable_sha256",
                    "python_executable_sha256", "runtime_environment"):
            if evidence[key] != old_environment[key]:
                raise ValueError(f"template feature repeat environment differs: {key}")
    for name, expected in sources.items():
        if native.sha(native.ROOT / name) != expected:
            raise ValueError(f"template feature input changed during capture: {name}")
    native.environment()
    native.write_json(output / "manifest.json", {
        "schema": "bcore.native-template-feature-capture.v2", "jar_sha256": native.JAR_SHA256,
        "reference_sha256": REFERENCE_SHA256,
        "source_sha256": sources, "historical_source_sha256": preserved,
        "sample_count": 153, "admission_count": 12, "potent_sulfur_load_count": 55,
        "independent_repeat": repeat is not None, "repeat_manifest_sha256": pin,
        "files_sha256": {path.relative_to(output).as_posix(): native.sha(path) for path in sorted(output.rglob("*"))
                         if path.is_file() and path.relative_to(output).parts[0] not in
                         (WORLD, "logs", "tmp", "crash-reports")},
    })
    verified(output, native.sha(output / "manifest.json"))
    print(json.dumps({"status": "pass", "output": str(output), "sample_count": 153,
                      "admission_count": 12, "potent_sulfur_load_count": 55,
                      "manifest_sha256": native.sha(output / "manifest.json")}, indent=2))


def promote(source: Path, pin: str) -> None:
    native.environment()
    manifest, _ = verified(source, pin)
    if not manifest["independent_repeat"]:
        raise ValueError("template feature promotion requires an independent repeat")
    destination = native.ROOT / "crates/bcore-worldgen/data" / FIXTURE
    expected = manifest["files_sha256"][FIXTURE]
    if destination.exists():
        if native.sha(destination) != expected:
            raise ValueError("different target template feature fixture already exists")
    else:
        with destination.open("xb") as stream:
            stream.write((source / FIXTURE).read_bytes())
    if native.sha(destination) != expected:
        raise ValueError("template feature publication readback differs")
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
