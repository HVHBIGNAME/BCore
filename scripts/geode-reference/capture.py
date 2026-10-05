"""Independent Minecraft 26.1 geode capture; shared probes/catalogs are read-only.

Run --inspect to retain native bytecode and the original geode resources under
the isolated build directory. Capture refuses to overwrite an existing fixture;
--verify re-runs native code and compares with an existing capture.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import zipfile


ROOT = Path(__file__).resolve().parents[2]
JAR = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
JAVA = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
BUILD = ROOT / "target/full-parity-20261002/geode-native"
EXPECTED_JAR = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
SOURCES = [
    ROOT / "scripts/TreeReference.java",
    ROOT / "scripts/NativeEntityLevel.java",
    ROOT / "scripts/NativeWorldgenRegistries.java",
    ROOT / "scripts/TreeEffectReference.java",
    ROOT / "scripts/geode-reference/GeodeReference.java",
]
CLASSES = [
    "net.minecraft.world.level.levelgen.feature.GeodeFeature",
    "net.minecraft.world.level.levelgen.feature.Feature",
    "net.minecraft.world.level.levelgen.feature.ConfiguredFeature",
    "net.minecraft.world.level.levelgen.feature.configurations.GeodeConfiguration",
    "net.minecraft.world.level.levelgen.GeodeBlockSettings",
    "net.minecraft.world.level.levelgen.GeodeLayerSettings",
    "net.minecraft.world.level.levelgen.GeodeCrackSettings",
    "net.minecraft.world.level.levelgen.synth.NormalNoise",
    "net.minecraft.world.level.levelgen.synth.PerlinNoise",
    "net.minecraft.world.level.levelgen.LegacyRandomSource",
    "net.minecraft.world.level.levelgen.WorldgenRandom",
    "net.minecraft.world.level.block.BuddingAmethystBlock",
    "net.minecraft.world.level.material.EmptyFluid",
    "net.minecraft.world.level.material.FlowingFluid",
    "net.minecraft.core.BlockPos",
    "net.minecraft.core.BlockPos$3",
    "net.minecraft.core.Vec3i",
    "net.minecraft.util.Mth",
    "net.minecraft.server.level.WorldGenRegion",
]
RESOURCES = [
    "data/minecraft/worldgen/configured_feature/amethyst_geode.json",
    "data/minecraft/worldgen/placed_feature/amethyst_geode.json",
    "data/minecraft/tags/block/geode_invalid_blocks.json",
    "data/minecraft/tags/block/features_cannot_replace.json",
]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def format_json(value, level=0):
    indent = "  " * level
    if isinstance(value, dict) and value:
        lines = [
            indent + "  " + json.dumps(key) + ": " + format_json(item, level + 1)
            for key, item in value.items()
        ]
        return "{\n" + ",\n".join(lines) + "\n" + indent + "}"
    if isinstance(value, list) and value and (
        len(value) > 16 or any(isinstance(item, (dict, list)) for item in value)
    ):
        lines = [indent + "  " + format_json(item, level + 1) for item in value]
        return "[\n" + ",\n".join(lines) + "\n" + indent + "]"
    return json.dumps(value, allow_nan=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inspect", action="store_true")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    jar_hash = sha(JAR.read_bytes())
    if jar_hash != EXPECTED_JAR:
        raise ValueError(f"Wrong native JAR: {jar_hash}")
    if not BUILD.parent.is_dir():
        raise ValueError(f"Missing expected build parent: {BUILD.parent}")
    BUILD.mkdir(exist_ok=True)
    if args.inspect:
        destination = BUILD / "bytecode"
        destination.mkdir(exist_ok=True)
        for name in CLASSES:
            run = subprocess.run(
                ["javap", "-classpath", str(JAR), "-c", "-p", name],
                check=True, capture_output=True, text=True,
            )
            path = destination / (name.rsplit(".", 1)[-1] + ".txt")
            path.write_text(run.stdout, encoding="utf-8")
            print(path.relative_to(ROOT))
        with zipfile.ZipFile(JAR) as jar:
            for name in RESOURCES:
                path = destination / (name.split("/")[-2] + "_" + name.split("/")[-1])
                path.write_bytes(jar.read(name))
                print(path.relative_to(ROOT))
        return
    if not args.output and not args.verify:
        parser.error("--output or --verify is required")
    if args.output and (args.output.exists() or not args.output.parent.is_dir()):
        parser.error("output must be new and its parent must already exist")
    subprocess.run(["javac", "-d", str(BUILD), *map(str, SOURCES)], check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [BUILD, JAR, *libraries]))
    command = [str(JAVA), "-Xmx2g", "-cp", classpath, "GeodeReference"]
    run = subprocess.run(command, cwd=BUILD, capture_output=True, text=True, timeout=480)
    if run.returncode:
        raise RuntimeError(run.stdout + "\n" + run.stderr)
    marker = "GEODE_REFERENCE="
    records = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise ValueError(f"Expected one capture: {run.stdout}")
    with zipfile.ZipFile(JAR) as jar:
        resource_hashes = {name: sha(jar.read(name)) for name in RESOURCES}
        class_hashes = {
            name: sha(jar.read(name.replace(".", "/") + ".class")) for name in CLASSES
        }
    result = {
        "minecraft": "26.1",
        "protocol": 775,
        "jar_sha256": jar_hash,
        "probe_sha256": sha(b"".join(path.read_bytes() for path in SOURCES)),
        "source_sha256": {
            path.relative_to(ROOT).as_posix(): sha(path.read_bytes())
            for path in [*SOURCES, Path(__file__)]
        },
        "resource_sha256": resource_hashes,
        "class_sha256": class_hashes,
        **json.loads(records[0]),
    }
    if args.verify:
        expected = json.loads(args.verify.read_text(encoding="utf-8"))
        if result != expected:
            raise ValueError(f"Independent native capture differs from {args.verify}")
        print(f"Verified independent native capture: {args.verify}")
    if args.output:
        args.output.write_text(format_json(result) + "\n", encoding="utf-8")
        print(f"Captured {len(result['samples'])} native cases: {args.output}")


if __name__ == "__main__":
    main()
