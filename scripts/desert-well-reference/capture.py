"""Pinned native desert-well archaeology and independent component capture."""
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
PIN = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
CLASSES = [
    "world.level.levelgen.feature.DesertWellFeature",
    "world.level.levelgen.feature.Feature",
    "world.level.levelgen.feature.ConfiguredFeature",
    "world.level.levelgen.placement.PlacedFeature",
    "world.level.block.entity.BrushableBlockEntity",
    "server.level.WorldGenRegion",
    "world.level.chunk.ProtoChunk",
    "core.BlockPos", "core.Direction$Plane",
]
RESOURCES = [
    "worldgen/configured_feature/desert_well.json",
    "worldgen/placed_feature/desert_well.json",
    "worldgen/biome/desert.json",
]
SOURCES = [ROOT / ("scripts/" + s) for s in [
    "TreeReference.java", "NativeEntityLevel.java", "NativeWorldgenRegistries.java",
    "TreeEffectReference.java", "geode-reference/GeodeReference.java",
    "desert-well-reference/DesertWellReference.java",
]]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--build", type=Path, required=True)
    p.add_argument("--inspect", action="store_true")
    p.add_argument("--output", type=Path)
    p.add_argument("--verify", type=Path)
    p.add_argument("--locate", action="store_true")
    a = p.parse_args()
    assert sha(JAR.read_bytes()) == PIN
    build = a.build.resolve()
    assert build.is_relative_to(ROOT / "target") and not build.exists()
    build.mkdir(parents=True)
    with zipfile.ZipFile(JAR) as jar:
        native = {n: sha(jar.read("net/minecraft/" + n.replace(".", "/") + ".class")) for n in CLASSES}
        resources = {n: sha(jar.read("data/minecraft/" + n)) for n in RESOURCES}
        for n in RESOURCES:
            (build / (n.replace("/", "_"))).write_bytes(jar.read("data/minecraft/" + n))
    if a.inspect:
        for n in CLASSES:
            run = subprocess.run(["javap", "-classpath", str(JAR), "-c", "-p", "net.minecraft." + n], capture_output=True, check=True)
            (build / (n + ".txt")).write_bytes(run.stdout)
        print(build)
        return
    for source in SOURCES:
        (build / source.name).write_bytes(source.read_bytes())
    (build / "capture.py").write_bytes(Path(__file__).read_bytes())
    subprocess.run(["javac", "-d", str(build), *map(str, SOURCES)], check=True)
    cp = os.pathsep.join(map(str, [build, JAR, *sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))]))
    command = [str(JAVA), "-Xmx2g", "-XX:ActiveProcessorCount=2", "-cp", cp, "DesertWellReference"]
    if a.locate:
        command.append("locate")
    run = subprocess.run(command, cwd=build, capture_output=True, timeout=900)
    (build / "stdout.log").write_bytes(run.stdout)
    (build / "stderr.log").write_bytes(run.stderr)
    assert run.returncode == 0, (run.stdout.decode(errors="replace"), run.stderr.decode(errors="replace"))
    lines = [s.split("DESERT_WELL_REFERENCE=", 1)[1] for s in run.stdout.decode().splitlines() if "DESERT_WELL_REFERENCE=" in s]
    assert len(lines) == 1
    result = {"minecraft": "26.1", "jar_sha256": PIN, "native_sha256": native,
              "resource_sha256": resources,
              "source_sha256": {s.relative_to(ROOT).as_posix(): sha(s.read_bytes()) for s in [*SOURCES, Path(__file__)]},
              **json.loads(lines[0])}
    (build / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    if a.verify:
        assert result == json.loads(a.verify.read_text(encoding="utf-8")), "native repeat differs"
        print("Independent native repeat matched", a.verify)
    if a.output:
        assert not a.output.exists() and a.output.parent.is_dir()
        a.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    print("Result:", build / "result.json", sha((build / "result.json").read_bytes()))


if __name__ == "__main__":
    main()
