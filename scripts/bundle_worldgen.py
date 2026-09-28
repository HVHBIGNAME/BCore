"""Bundle pinned 26.1 worldgen data and the server's synchronized biome ID map.

First capture --probe worldgen_data with capture_tree_reference.py. Climate rows
and noise assets must come from the same JAR. The configuration capture defines
BCore's wire/save IDs, which may differ from vanilla's internal registry order.
"""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

from extract_biomes import packets, read_string, read_varint, skip_nbt

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vanilla-jar", type=Path, default=ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar")
    parser.add_argument("--reference", type=Path, default=ROOT / "target/worldgen-data-reference.json")
    parser.add_argument("--registry", type=Path, default=ROOT / "crates/bcore-protocol/data/config_packets.bin")
    parser.add_argument("--output", type=Path, default=ROOT / "crates/bcore-worldgen/data/vanilla_worldgen.json")
    parser.add_argument("--verify", action="store_true", help="compare without replacing the bundle")
    args = parser.parse_args()
    reference = json.loads(args.reference.read_text(encoding="utf-8"))
    jar_hash = hashlib.sha256(args.vanilla_jar.read_bytes()).hexdigest()
    if reference["minecraft"] != "26.1" or reference["jar_sha256"] != jar_hash:
        raise ValueError("climate reference must match the target 26.1 JAR")
    assets = {}
    prefix = "data/minecraft/worldgen/"
    with zipfile.ZipFile(args.vanilla_jar) as jar:
        for path in sorted(jar.namelist()):
            relative = path.removeprefix(prefix)
            if path.startswith(prefix) and relative.split("/", 1)[0] in {"density_function", "noise", "noise_settings"} and path.endswith(".json"):
                assets[relative] = json.loads(jar.read(path))
    assets["biome_parameters/overworld.json"] = reference["parameters"]
    assets["worldgen_version.json"] = {"minecraft": "26.1", "jar_sha256": jar_hash,
                                      "climate_probe_sha256": reference["probe_sha256"]}
    for _, payload in packets(args.registry.read_bytes()):
        try:
            name, at = read_string(payload, 0)
        except (ValueError, IndexError, UnicodeDecodeError):
            continue
        if name != "minecraft:worldgen/biome":
            continue
        count, at = read_varint(payload, at)
        biomes = []
        for _ in range(count):
            name, at = read_string(payload, at)
            has_data = payload[at]
            at += 1
            if has_data:
                at = skip_nbt(payload, at)
            biomes.append(name)
        if at != len(payload):
            raise ValueError("trailing biome registry data")
        assets["biome_registry.json"] = biomes
        break
    if "biome_registry.json" not in assets:
        raise ValueError("biome registry not found")
    missing = {row["biome"] for row in assets["biome_parameters/overworld.json"]["biomes"]} - set(biomes)
    if missing:
        raise ValueError(f"parameter biomes absent from registry: {missing}")
    if args.verify:
        if json.loads(args.output.read_text(encoding="utf-8")) != assets:
            raise ValueError(f"bundle differs from native inputs: {args.output}")
        print(f"Verified {len(assets)} assets against the pinned 26.1 JAR")
        return
    args.output.write_text(json.dumps(assets, separators=(",", ":"), sort_keys=True) + "\n", encoding="utf-8")
    print(f"Bundled {len(assets)} assets, {len(biomes)} biomes: {args.output.stat().st_size} bytes")
    print(f"sha256: {hashlib.sha256(args.output.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main()
