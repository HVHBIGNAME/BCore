"""Native named-density identity witness and current expanded-reference inventory."""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build", type=Path, required=True)
    destination = parser.add_mutually_exclusive_group(required=True)
    destination.add_argument("--output", type=Path)
    destination.add_argument("--verify", type=Path)
    args = parser.parse_args()
    build = args.build.resolve()
    build.mkdir(parents=True, exist_ok=False)
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    jar_hash = hashlib.sha256(jar.read_bytes()).hexdigest()
    assert jar_hash == JAR_SHA256, jar_hash
    assets = json.loads((ROOT / "crates/bcore-worldgen/data/vanilla_worldgen.json").read_text())
    router = assets["noise_settings/overworld.json"]["noise_router"]
    inventory = {}
    with zipfile.ZipFile(jar) as archive:
        for field in ("final_density", "preliminary_surface_level", "depth"):
            refs = Counter()
            def visit(value):
                if isinstance(value, str):
                    path = "density_function/" + value.removeprefix("minecraft:") + ".json"
                    if path in assets:
                        assert assets[path] == json.loads(archive.read("data/minecraft/worldgen/" + path))
                        refs[path] += 1
                        visit(assets[path])
                elif isinstance(value, dict):
                    for key, child in value.items():
                        if key not in ("type", "noise", "rarity_value_mapper"):
                            visit(child)
                elif isinstance(value, list):
                    for child in value:
                        visit(child)
            visit(router[field])
            inventory[field] = {"expanded_named_loads": sum(refs.values()), "unique_names": len(refs), "loads_by_name": dict(refs)}
            print(field, inventory[field], flush=True)
    (build / "expanded-inventory.json").write_text(json.dumps(inventory, indent=2))
    original = ROOT / "crates/bcore-worldgen/src/density.rs"
    (build / "density-source.rs").write_bytes(original.read_bytes())
    sources = [ROOT / p for p in ("scripts/TreeReference.java", "scripts/NativeWorldgenRegistries.java",
        "scripts/jigsaw-reference/JigsawSupport.java", "scripts/density-materials-reference/DensityMaterialsReference.java",
        "scripts/density-sharing-reference/DensitySharingReference.java")]
    frozen = []
    for source in sources:
        snapshot = build / source.name
        snapshot.write_bytes(source.read_bytes())
        frozen.append(snapshot)
    compiled = subprocess.run(["javac", "-d", str(build), *map(str, frozen)], capture_output=True, text=True)
    (build / "javac.log").write_text(compiled.stdout + compiled.stderr)
    compiled.check_returncode()
    cp = os.pathsep.join(map(str, [build, jar, *sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))]))
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    result = subprocess.run([str(java), "-Xmx1G", "-cp", cp, "DensitySharingReference"], cwd=build,
                            capture_output=True, text=True, timeout=900)
    (build / "native.stdout.log").write_text(result.stdout)
    (build / "native.stderr.log").write_text(result.stderr)
    result.check_returncode()
    marker = "DENSITY_SHARING_REFERENCE="
    records = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    assert len(records) == 1
    data = {"minecraft": "26.1", "protocol": 775, "jar_sha256": jar_hash,
            "sources": {p.relative_to(ROOT).as_posix(): hashlib.sha256(f.read_bytes()).hexdigest() for p, f in zip(sources, frozen)},
            **json.loads(records[0])}
    evidence = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in build.iterdir() if p.is_file()}
    (build / "sha256.json").write_text(json.dumps(evidence, indent=2))
    if args.verify:
        assert data == json.loads(args.verify.read_text(encoding="utf-8")), "native recapture differs"
        print(f"Verified {args.verify}")
        return
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(data, output, indent=2)
        output.write("\n")
    print("fixture sha256=" + hashlib.sha256(args.output.read_bytes()).hexdigest())
    for case in data["cases"]:
        print(case["name"], "holder/value:", case["holders_same"], case["values_same"],
              "mapped/wrapped:", [(c["mapped_arguments_same"], c["wrapped_arguments_same"]) for c in case["seeded"]])


if __name__ == "__main__":
    main()
