"""Run isolated worldgen probes against the local vanilla 26.1 JAR.

Compiles the reflection-only probe with javac (21+), then runs it using Java 25+.
Loads configurations and registries from the JAR and calls the selected native
implementation. Each fixture describes its controlled world and checked outputs;
component captures do not establish the complete world-generation schedule.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java", required=True, type=Path)
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--vanilla", type=Path, default=ROOT / "target/vanilla-775")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--build-dir", type=Path, help="isolated Java build directory for parallel probes")
    parser.add_argument("--probe", choices=["tree", "heightmap", "fallen", "structure", "random", "ore", "ore_placement", "biome_zoom", "aquifer", "feature_order", "monster_room", "mineshaft", "mineshaft_blocks", "mineshaft_start", "mineshaft_region", "worldgen_data", "numeric", "entity_packet", "carver", "mth", "vegetation", "standing", "standing_blocks"], default="tree")
    parser.add_argument("--input", type=Path, help="numeric probe requests (omit for the standard matrix)")
    parser.add_argument("--verify", type=Path, help="compare the result with a checked-in fixture")
    args = parser.parse_args()
    if args.input and args.probe != "numeric":
        parser.error("--input is supported only by --probe numeric")
    jar = args.vanilla / "versions/26.1/server-26.1.jar"
    if not jar.is_file():
        parser.error(f"missing 26.1 server JAR: {jar}")
    build = (args.build_dir or ROOT / "target/tree-reference").resolve()
    build.mkdir(parents=True, exist_ok=True)
    source = ROOT / "scripts/TreeReference.java"
    sources = [source]
    main_class = "TreeReference"
    if args.probe == "heightmap":
        main_class = "HeightmapReference"
        sources.append(ROOT / "scripts/HeightmapReference.java")
    elif args.probe == "fallen":
        main_class = "FallenTreeReference"
        sources.append(ROOT / "scripts/FallenTreeReference.java")
    elif args.probe == "structure":
        main_class = "StructureReference"
        sources.append(ROOT / "scripts/StructureReference.java")
    elif args.probe == "random":
        main_class = "RandomReference"
        sources.append(ROOT / "scripts/RandomReference.java")
    elif args.probe == "ore":
        main_class = "OreReference"
        sources.append(ROOT / "scripts/OreReference.java")
    elif args.probe == "ore_placement":
        main_class = "OrePlacementReference"
        sources.extend([ROOT / "scripts/OreReference.java", ROOT / "scripts/FeatureOrderReference.java", ROOT / "scripts/OrePlacementReference.java"])
    elif args.probe == "biome_zoom":
        main_class = "BiomeZoomReference"
        sources.append(ROOT / "scripts/BiomeZoomReference.java")
    elif args.probe == "aquifer":
        main_class = "AquiferReference"
        sources.extend([ROOT / "scripts/OreReference.java", ROOT / "scripts/AquiferReference.java"])
    elif args.probe == "feature_order":
        main_class = "FeatureOrderReference"
        sources.append(ROOT / "scripts/FeatureOrderReference.java")
    elif args.probe == "monster_room":
        main_class = "MonsterRoomReference"
        sources.append(ROOT / "scripts/MonsterRoomReference.java")
    elif args.probe == "mineshaft":
        main_class = "MineshaftReference"
        sources.append(ROOT / "scripts/MineshaftReference.java")
    elif args.probe == "mineshaft_blocks":
        main_class = "MineshaftBlockReference"
        sources.extend([ROOT / "scripts/OreReference.java",ROOT / "scripts/NativeEntityLevel.java",ROOT / "scripts/MineshaftBlockReference.java"])
    elif args.probe == "mineshaft_start":
        main_class = "MineshaftStartReference"
        sources.extend([ROOT / "scripts/OreReference.java",ROOT / "scripts/NativeWorldgenRegistries.java",ROOT / "scripts/MineshaftStartReference.java"])
    elif args.probe == "worldgen_data":
        main_class = "WorldgenDataReference"
        sources.extend([ROOT / "scripts/NativeWorldgenRegistries.java",ROOT / "scripts/WorldgenDataReference.java"])
    elif args.probe == "mineshaft_region":
        main_class = "MineshaftRegionReference"
        sources.extend([ROOT / "scripts/OreReference.java",ROOT / "scripts/NativeEntityLevel.java",ROOT / "scripts/MineshaftBlockReference.java",ROOT / "scripts/NativeWorldgenRegistries.java",ROOT / "scripts/MineshaftStartReference.java",ROOT / "scripts/MineshaftRegionReference.java"])
    elif args.probe == "numeric":
        main_class = "NumericReference"
        sources.extend([ROOT / "scripts/NativeWorldgenRegistries.java", ROOT / "scripts/NumericReference.java"])
    elif args.probe == "entity_packet":
        main_class = "EntityPacketReference"
        sources.extend([ROOT / "scripts/NativeEntityLevel.java", ROOT / "scripts/EntityPacketReference.java"])
    elif args.probe == "carver":
        main_class = "CarverReference"
        sources.extend([ROOT / "scripts/OreReference.java", ROOT / "scripts/NativeWorldgenRegistries.java", ROOT / "scripts/NativeEntityLevel.java", ROOT / "scripts/CarverReference.java"])
    elif args.probe == "mth":
        main_class = "MthReference"
        sources.append(ROOT / "scripts/MthReference.java")
    elif args.probe in ("vegetation", "standing", "standing_blocks"):
        main_class = "VegetationReference"
        vegetation_source = (ROOT / "scripts/vegetation-bounded/VegetationReference.java"
                             if args.probe == "vegetation" else ROOT / "scripts/VegetationReference.java")
        sources.extend([ROOT / "scripts/OreReference.java", ROOT / "scripts/NativeWorldgenRegistries.java", vegetation_source])
    subprocess.run([args.javac, "-d", str(build), *map(str, sources)], check=True)
    classpath = os.pathsep.join(map(str, [build, jar, *sorted((args.vanilla / "libraries").rglob("*.jar"))]))
    probe_args = []
    if args.probe == "numeric":
        if args.input:
            probe_args.append(str(args.input.resolve()))
        else:
            from numeric_cases import requests
            input_path = build / "numeric-requests.json"
            input_path.write_text(json.dumps(requests(jar), allow_nan=False), encoding="utf-8")
            probe_args.append(str(input_path))
    probe_env = os.environ.copy()
    if args.probe == "standing_blocks":
        probe_env["BCORE_STANDING_PROBE"] = "blocks"
    elif args.probe == "standing":
        probe_env.pop("BCORE_STANDING_PROBE", None)
    result = subprocess.run([str(args.java.resolve()), "-cp", classpath, main_class, *probe_args],
                            cwd=build, env=probe_env, capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    marker = ("VEGETATION" if args.probe in ("standing", "standing_blocks")
              else args.probe.upper()) + "_REFERENCE="
    records = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise ValueError(f"expected one probe record: {result.stdout}")
    reference = {"minecraft": "26.1", "jar_sha256": hashlib.sha256(jar.read_bytes()).hexdigest(),
                 "probe_sha256": hashlib.sha256(b"".join(source.read_bytes() for source in sources)).hexdigest()}
    payload = json.loads(records[0])
    if args.probe in ("tree", "fallen"):
        reference["samples"] = payload
    else:
        reference.update(payload)
    if args.verify:
        expected = json.loads(args.verify.read_text(encoding="utf-8"))
        if reference != expected:
            raise ValueError(f"JAR output differs from {args.verify}")
    args.output.write_text(json.dumps(reference, indent=2) + "\n", encoding="utf-8")
    if args.probe == "numeric":
        count = sum(len(sample["bits"]) for sample in reference["samples"])
        print(f"{'Verified' if args.verify else 'Captured'} {count} numeric values in {len(reference['samples'])} cases to {args.output}")
    elif args.probe == "worldgen_data":
        print(f"Captured {len(reference['parameters']['biomes'])} climate rows and {len(reference['biome_registry'])} native biome ids to {args.output}")
    elif args.probe == "standing_blocks":
        print(f"{'Verified' if args.verify else 'Captured'} {reference['state_count']} native standing-tree block states to {args.output}")
    elif args.verify and args.probe == "feature_order":
        count = sum(map(len, reference["steps"]))
        print(f"Verified {count} feature slots in {len(reference['steps'])} steps for {len(reference['possible_biomes'])} biomes against {args.verify}")
    elif args.verify:
        print(f"Verified {len(reference['samples'])} samples against {args.verify}")
    elif args.probe in ("ore", "ore_placement", "aquifer", "monster_room", "mineshaft", "mineshaft_blocks", "mineshaft_start", "mineshaft_region", "entity_packet", "carver", "mth", "vegetation", "standing"):
        print(f"Captured {len(reference['samples'])} native {args.probe} samples to {args.output}")
    else:
        print(json.dumps(reference, indent=2) if args.probe == "tree" else json.dumps(reference))


if __name__ == "__main__":
    main()
