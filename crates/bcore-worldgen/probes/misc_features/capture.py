"""Independent 26.1 misc-feature oracle; never overwrites a differing fixture."""

import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import zipfile


ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
VANILLA = ROOT / "target/vanilla-775"
JAR = VANILLA / "versions/26.1/server-26.1.jar"
JAVA = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
PINNED_JAR = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    text = json.dumps(value, indent=2, ensure_ascii=False) + "\n"
    if path.exists():
        if json.loads(path.read_text(encoding="utf-8")) != value:
            raise ValueError(f"Refusing to replace differing native fixture: {path}")
        print(f"Verified identical: {path}")
    else:
        if not path.parent.is_dir():
            raise ValueError(f"Output parent does not exist: {path.parent}")
        path.write_text(text, encoding="utf-8")
        print(f"Captured: {path}")


def inventory():
    catalog_path = ROOT / "crates/bcore-worldgen/data/base_feature_catalog_26_1.json"
    catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
    docs = defaultdict(dict)
    with zipfile.ZipFile(JAR) as jar:
        for name in jar.namelist():
            match = re.fullmatch(r"data/minecraft/worldgen/(configured_feature|placed_feature|biome)/(.+)\.json", name)
            if match:
                docs[match[1]]["minecraft:" + match[2]] = json.loads(jar.read(name))
    types = defaultdict(list)
    for name, doc in docs["configured_feature"].items():
        types[doc["type"]].append(name)
    roots = sorted({name for biome in catalog["overworld_biomes"]
                    for step in docs["biome"][biome]["features"] for name in step})
    reached = defaultdict(set)
    configured_names = set()

    def placed(ref, path):
        doc = docs["placed_feature"][ref] if isinstance(ref, str) else ref
        configured(doc["feature"], path)

    def configured(ref, path):
        if isinstance(ref, str):
            configured_names.add(ref)
            doc = docs["configured_feature"][ref]
        else:
            doc = ref
        kind = doc["type"]
        reached[kind].add(path)
        config = doc["config"]
        if kind == "minecraft:random_boolean_selector":
            placed(config["feature_true"], path)
            placed(config["feature_false"], path)
        elif kind == "minecraft:simple_random_selector":
            for child in config["features"]:
                placed(child, path)
        elif kind == "minecraft:random_selector":
            for child in config["features"]:
                placed(child["feature"], path)
            placed(config["default"], path)
        elif kind in ("minecraft:root_system", "minecraft:random_patch", "minecraft:flower", "minecraft:no_bonemeal_flower"):
            placed(config["feature"], path)
        elif kind in ("minecraft:vegetation_patch", "minecraft:waterlogged_vegetation_patch"):
            placed(config["vegetation_feature"], path)

    for root in roots:
        placed(root, root)
    return {
        "minecraft": "26.1", "protocol": 775, "jar_sha256": sha(JAR),
        "inventory_script_sha256": sha(Path(__file__)),
        "native_overworld_biome_catalog_sha256": sha(catalog_path),
        "configured_by_type": {k: sorted(v) for k, v in sorted(types.items())},
        "overworld_roots_by_type": {k: sorted(v) for k, v in sorted(reached.items())},
        "overworld_configured": sorted(configured_names),
        "overworld_placed_roots": roots,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("metadata", "samples", "math", "inventory", "bytecode"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    parser.add_argument("--build", type=Path,
                        default=ROOT / "target/full-parity-20261002/native-misc-features")
    parser.add_argument("--suite", default="priority")
    parser.add_argument("--class", dest="classes", action="append", default=[])
    parser.add_argument("--method", action="append", default=[])
    args = parser.parse_args()
    args.build = args.build.resolve()
    if sha(JAR) != PINNED_JAR:
        raise ValueError("Native JAR differs from the pinned 26.1 oracle")
    if args.mode == "bytecode":
        for cls in args.classes:
            result = subprocess.run(["javap", "-classpath", str(JAR), "-c", "-p", "net.minecraft." + cls],
                                    check=True, capture_output=True, text=True)
            if not args.method:
                print(result.stdout)
            else:
                print(cls)
                for block in result.stdout.split("\n\n"):
                    signature = block.splitlines()[0] if block.splitlines() else ""
                    if any(re.search(r"\b" + re.escape(method) + r"\(", signature) for method in args.method):
                        print(block + "\n")
        return
    if args.mode == "inventory":
        reference = inventory()
        print("All configured feature types:", len(reference["configured_by_type"]))
        for kind, roots in reference["overworld_roots_by_type"].items():
            print(f"{kind}: {len(roots)} overworld roots")
    else:
        args.build.mkdir(parents=True, exist_ok=True)
        sources = [ROOT / "scripts" / name for name in
                   ("TreeReference.java", "NativeWorldgenRegistries.java", "BaseFeatureReference.java")]
        sources.append(HERE / "MiscFeatureReference.java")
        snapshots = args.build / "sources"
        snapshots.mkdir(exist_ok=True)
        for source in [*sources, Path(__file__)]:
            target = snapshots / source.name
            content = source.read_bytes()
            if target.exists() and target.read_bytes() != content:
                raise ValueError(f"Use a fresh --build to preserve the earlier source snapshot: {args.build}")
            target.write_bytes(content)
        subprocess.run(["javac", "-d", str(args.build), *map(str, sources)], check=True)
        classpath = os.pathsep.join(map(str, [args.build, JAR, *sorted((VANILLA / "libraries").rglob("*.jar"))]))
        result = subprocess.run([str(JAVA), "-Xmx1G", "-cp", classpath, "MiscFeatureReference",
                                 args.mode, str(JAR), args.suite], cwd=args.build,
                                capture_output=True, text=True, timeout=600)
        (args.build / f"{args.mode}-{args.suite}.stdout.log").write_text(result.stdout, encoding="utf-8")
        (args.build / f"{args.mode}-{args.suite}.stderr.log").write_text(result.stderr, encoding="utf-8")
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        marker = "MISC_FEATURE_REFERENCE="
        records = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
        if len(records) != 1:
            raise RuntimeError("Missing oracle record; see preserved native logs")
        reference = {
            "minecraft": "26.1", "protocol": 775, "jar_sha256": sha(JAR),
            "sources_sha256": {str(p.relative_to(ROOT)).replace("\\", "/"): sha(p) for p in sources},
            "capture_script_sha256": sha(Path(__file__)), "suite": args.suite,
            **json.loads(records[0]),
        }
        print(f"Native {args.mode}: {len(reference.get('samples', []))} samples")
    if args.verify:
        expected = json.loads(args.verify.read_text(encoding="utf-8"))
        # Source hashes describe the capture revision, not the feature output.
        keys = ("sources_sha256", "capture_script_sha256", "inventory_script_sha256")
        if {k: v for k, v in reference.items() if k not in keys} != {k: v for k, v in expected.items() if k not in keys}:
            raise ValueError(f"Native output differs from {args.verify}")
        print(f"Verified native results: {args.verify}")
    if args.output:
        save(args.output, reference)


if __name__ == "__main__":
    main()
