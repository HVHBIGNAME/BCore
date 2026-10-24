"""Authenticate and inspect the prepared 26.2 artifact through the heavy-work queue."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
PREPARATION = ROOT / "target/migration-26_2-prepare-20261021-01/results.json"
JAVA = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
JAR = ROOT / "target/vanilla-776/versions/26.2/server-26.2.jar"
JAR_SHA256 = "183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8"

INSPECT_CLASSES = (
    "net.minecraft.network.protocol.login.ClientboundLoginFinishedPacket",
    "net.minecraft.network.protocol.game.ClientboundLoginPacket",
    "net.minecraft.core.registries.BuiltInRegistries",
    "net.minecraft.core.Registry",
    "net.minecraft.core.registries.Registries",
    "net.minecraft.world.entity.EntityType",
    "net.minecraft.world.level.block.Block",
    "net.minecraft.world.level.biome.Climate$ParameterPoint",
    "net.minecraft.world.level.levelgen.DensityFunctions$IntervalSelect",
    "net.minecraft.world.level.levelgen.NoiseChunk$BlendDensity",
    "net.minecraft.world.level.levelgen.feature.SequenceFeature",
    "net.minecraft.world.level.levelgen.feature.WeightedRandomSelectorFeature",
    "net.minecraft.world.level.levelgen.feature.TemplateFeature",
    "net.minecraft.world.level.levelgen.feature.SpeleothemFeature",
    "net.minecraft.world.level.levelgen.feature.SpeleothemClusterFeature",
    "net.minecraft.world.level.levelgen.feature.configurations.TreeConfiguration",
    "net.minecraft.world.level.levelgen.feature.featuresize.TwoLayersFeatureSize",
    "net.minecraft.world.level.levelgen.feature.featuresize.ThreeLayersFeatureSize",
    "net.minecraft.world.level.levelgen.feature.configurations.CompositeFeatureConfiguration",
    "net.minecraft.world.level.levelgen.feature.configurations.WeightedRandomFeatureConfiguration",
    "net.minecraft.util.random.WeightedList",
    "net.minecraft.util.random.WeightedList$Compact",
    "net.minecraft.util.random.WeightedList$Flat",
    "net.minecraft.world.level.levelgen.feature.configurations.TemplateFeatureConfiguration",
    "net.minecraft.world.level.levelgen.feature.configurations.TemplateFeatureConfiguration$TemplateEntry",
    "net.minecraft.world.level.levelgen.feature.configurations.SpeleothemConfiguration",
    "net.minecraft.world.level.levelgen.feature.configurations.SpeleothemClusterConfiguration",
    "net.minecraft.world.level.levelgen.feature.SpeleothemUtils",
    "net.minecraft.world.level.levelgen.blockpredicates.MatchingBiomesPredicate",
    "net.minecraft.world.level.levelgen.feature.LakeFeature",
    "net.minecraft.world.level.levelgen.feature.LakeFeature$Configuration",
    "net.minecraft.world.level.levelgen.SurfaceSystem",
    "net.minecraft.world.level.levelgen.feature.OreFeature",
    "net.minecraft.world.level.levelgen.feature.configurations.OreConfiguration",
    "net.minecraft.world.level.block.SpeleothemBlock",
    "net.minecraft.world.level.block.SulfurSpikeBlock",
    "net.minecraft.world.level.block.PotentSulfurBlock",
    "net.minecraft.world.level.block.state.properties.SpeleothemThickness",
    "net.minecraft.world.level.block.state.properties.PotentSulfurState",
)


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path: Path, value) -> None:
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def environment() -> tuple[dict, list[Path]]:
    if os.environ.get("CARGO_TARGET_DIR") != str(ROOT / "target/build"):
        raise ValueError("run through scripts/run_generation_check.py")
    prepared = json.loads(PREPARATION.read_bytes())
    if prepared["server_jar_sha256"] != JAR_SHA256 or sha(JAR) != JAR_SHA256:
        raise ValueError("official 26.2 JAR digest differs")
    libraries = []
    for name, expected in prepared["files_sha256"].items():
        path = ROOT / name
        if sha(path) != expected:
            raise ValueError(f"prepared artifact changed: {name}")
        if name.startswith("target/vanilla-776/libraries/"):
            libraries.append(path)
    if set(libraries) != set((ROOT / "target/vanilla-776/libraries").rglob("*.jar")):
        raise ValueError("library directory differs from official bundle membership")
    with zipfile.ZipFile(JAR) as archive:
        version = json.loads(archive.read("version.json"))
    if (version["name"], version["protocol_version"], version["world_version"]) != ("26.2", 776, 4903):
        raise ValueError("target version identity differs")
    return version, sorted(libraries)


def execute(command: list[str], output: Path, name: str, *, timeout: int = 300) -> str:
    completed = subprocess.run(command, cwd=output, capture_output=True, text=True,
                               encoding="utf-8", errors="strict", timeout=timeout)
    text = completed.stdout + completed.stderr
    with (output / f"{name}.log").open("x", encoding="utf-8") as stream:
        stream.write(text)
    write_json(output / f"{name}.command.json", {"command": command, "exit_code": completed.returncode})
    completed.check_returncode()
    return completed.stdout


def capture_probe(output: Path, libraries: list[Path], name: str, sources: list[Path], args=()) -> dict:
    build = output / name
    build.mkdir()
    for source in sources:
        shutil.copyfile(source, build / source.name)
    javac = shutil.which("javac")
    if javac is None:
        raise ValueError("javac is missing")
    execute([javac, "--release", "21", "-d", str(build),
             *[str(build / source.name) for source in sources]], output, f"{name}-compile")
    classpath = os.pathsep.join(map(str, [build, JAR, *libraries]))
    stdout = execute([str(JAVA), "-Xmx1G", "-XX:ActiveProcessorCount=2", "-cp", classpath,
                      sources[-1].stem, *map(str, args)], output, f"{name}-capture", timeout=600)
    marker = "MIGRATION_REFERENCE="
    records = [line.split(marker, 1)[1] for line in stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise ValueError(f"expected one {name} native result")
    result = {"minecraft": "26.2", "protocol": 776, "jar_sha256": JAR_SHA256,
              "probe_sha256": hashlib.sha256(b"".join((build / p.name).read_bytes() for p in sources)).hexdigest(),
              **json.loads(records[0])}
    write_json(output / f"{name}.json", result)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("inspect", "core"))
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--class", dest="classes", action="append",
                        help="inspect only this class; repeat to select additional classes")
    args = parser.parse_args()
    if args.classes and args.mode != "inspect":
        parser.error("--class applies only to inspect")
    classes = args.classes or list(INSPECT_CLASSES)
    if len(classes) != len(set(classes)) or any(
            re.fullmatch(r"(?:[A-Za-z_$][A-Za-z0-9_$]*\.)*[A-Za-z_$][A-Za-z0-9_$]*", name) is None
            for name in classes):
        parser.error("inspection classes must be unique Java binary names")
    output = args.output.resolve()
    if output.exists() or not output.is_relative_to(ROOT / "target"):
        parser.error("output must be a new directory under target/")
    version, libraries = environment()
    output.mkdir(parents=True)
    shutil.copyfile(__file__, output / "native.py")
    execute([str(JAVA), "-version"], output, "java-version")
    javap = shutil.which("javap")
    if javap is None:
        raise ValueError("javap is missing")
    captures = {}
    if args.mode == "inspect":
        for name in classes:
            execute([javap, "-classpath", str(JAR), "-c", "-p", name], output, name)
    else:
        sources = [ROOT / "scripts/TreeReference.java", ROOT / "scripts/NativeWorldgenRegistries.java",
                   HERE / "RegistryReference.java"]
        result = capture_probe(output, libraries, "registries", sources)
        captures = {"block_states": len(result["states"]),
                    "registries": {name: len(rows) for name, rows in result["registries"].items()},
                    "climate_rows": len(result["climate_rows"])}
        datagen = output / "datagen"
        classpath = os.pathsep.join(map(str, [JAR, *libraries]))
        execute([str(JAVA), "-Xmx1G", "-XX:ActiveProcessorCount=2", "-cp", classpath,
                 "net.minecraft.data.Main", "--reports", "--output", str(datagen)],
                output, "datagen", timeout=600)
    write_json(output / "results.json", {"version": version, "jar_sha256": JAR_SHA256,
                "java_executable_sha256": sha(JAVA), "java_version_log_sha256": sha(output / "java-version.log"),
                "libraries": {p.relative_to(ROOT).as_posix(): sha(p) for p in libraries}, "captures": captures,
                "inspected_classes": {name: sha(output / f"{name}.log") for name in classes}
                if args.mode == "inspect" else {}})
    print(json.dumps({"output": str(output), "mode": args.mode, "captures": captures}, indent=2), flush=True)


if __name__ == "__main__":
    main()
