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

# Dependencies follow TreeReference.java in this order for fixture provenance.
PROBES = {
    "tree": (),
    "heightmap": ("HeightmapReference.java",),
    "fallen": ("FallenTreeReference.java",),
    "structure": ("StructureReference.java",),
    "random": ("RandomReference.java",),
    "ore": ("OreReference.java",),
    "ore_placement": ("OreReference.java", "FeatureOrderReference.java", "OrePlacementReference.java"),
    "biome_zoom": ("BiomeZoomReference.java",),
    "aquifer": ("OreReference.java", "AquiferReference.java"),
    "feature_order": ("FeatureOrderReference.java",),
    "monster_room": ("MonsterRoomReference.java",),
    "mineshaft": ("MineshaftReference.java",),
    "mineshaft_blocks": ("OreReference.java", "NativeEntityLevel.java", "MineshaftBlockReference.java"),
    "mineshaft_start": ("OreReference.java", "NativeWorldgenRegistries.java", "MineshaftStartReference.java"),
    "mineshaft_region": ("OreReference.java", "NativeEntityLevel.java", "MineshaftBlockReference.java",
                         "NativeWorldgenRegistries.java", "MineshaftStartReference.java", "MineshaftRegionReference.java"),
    "worldgen_data": ("NativeWorldgenRegistries.java", "WorldgenDataReference.java"),
    "numeric": ("NativeWorldgenRegistries.java", "NumericReference.java"),
    "entity_packet": ("NativeEntityLevel.java", "EntityPacketReference.java"),
    "carver": ("OreReference.java", "NativeWorldgenRegistries.java", "NativeEntityLevel.java", "CarverReference.java"),
    "mth": ("MthReference.java",),
    "vegetation": ("OreReference.java", "NativeWorldgenRegistries.java", "vegetation-bounded/VegetationReference.java"),
    "standing": ("OreReference.java", "NativeWorldgenRegistries.java", "VegetationReference.java"),
    "standing_blocks": ("OreReference.java", "NativeWorldgenRegistries.java", "VegetationReference.java"),
    "beehive": ("NativeWorldgenRegistries.java", "BeehiveReference.java"),
    "tree_effect": ("NativeEntityLevel.java", "NativeWorldgenRegistries.java", "TreeEffectReference.java"),
    "tick": ("TickReference.java",),
    "feature_dependency": ("FeatureDependencyReference.java",),
}

SUMMARIES = {
    "numeric": lambda ref: f"{sum(len(s['bits']) for s in ref['samples'])} numeric values in {len(ref['samples'])} cases",
    "worldgen_data": lambda ref: f"{len(ref['parameters']['biomes'])} climate rows and {len(ref['biome_registry'])} native biome ids",
    "standing_blocks": lambda ref: f"{ref['state_count']} native standing-tree block states",
    "feature_order": lambda ref: f"{sum(map(len, ref['steps']))} feature slots in {len(ref['steps'])} steps for {len(ref['possible_biomes'])} biomes",
    "feature_dependency": lambda ref: f"{len(ref['statuses'])} native chunk statuses and their stage dependencies",
}


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java", required=True, type=Path)
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--vanilla", type=Path, default=ROOT / "target/vanilla-775")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--build-dir", type=Path, help="isolated Java build directory for parallel probes")
    parser.add_argument("--probe", choices=PROBES, default="tree")
    parser.add_argument("--input", type=Path, help="numeric probe requests (omit for the standard matrix)")
    parser.add_argument("--verify", type=Path, help="compare the result with a checked-in fixture")
    args = parser.parse_args()
    if args.input and args.probe != "numeric":
        parser.error("--input is supported only by --probe numeric")
    jar = args.vanilla / "versions/26.1/server-26.1.jar"
    if not jar.is_file():
        parser.error(f"missing 26.1 server JAR: {jar}")
    return args


def capture(args):
    jar = args.vanilla / "versions/26.1/server-26.1.jar"
    build = (args.build_dir or ROOT / "target/tree-reference").resolve()
    build.mkdir(parents=True, exist_ok=True)
    sources = [ROOT / "scripts" / source for source in ("TreeReference.java", *PROBES[args.probe])]
    main_class = sources[-1].stem
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
    return reference


def summarize(probe, reference):
    if formatter := SUMMARIES.get(probe):
        return formatter(reference)
    return f"{len(reference['samples'])} native {probe} samples"


def main():
    args = parse_args()
    reference = capture(args)
    if args.verify:
        expected = json.loads(args.verify.read_text(encoding="utf-8"))
        if reference != expected:
            raise ValueError(f"JAR output differs from {args.verify}")
    args.output.write_text(json.dumps(reference, indent=2) + "\n", encoding="utf-8")
    print(f"{'Verified' if args.verify else 'Captured'} {summarize(args.probe, reference)} to {args.output}")


if __name__ == "__main__":
    main()
