"""Bundle extracted vanilla worldgen JSON and the captured biome registry.

Inputs are the existing vanilla data-generator reports, unpacked server datapack,
and configuration packet capture. No values are inferred or synthesized.
"""
import argparse
import hashlib
import json
from pathlib import Path

from extract_biomes import packets, read_string, read_varint, skip_nbt

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--datapack", type=Path, default=ROOT / "target/datapack")
    parser.add_argument("--parameters", type=Path, default=ROOT / "target/datagen/reports/biome_parameters/minecraft/overworld.json")
    parser.add_argument("--registry", type=Path, default=ROOT / "crates/bcore-protocol/data/config_packets.bin")
    parser.add_argument("--output", type=Path, default=ROOT / "crates/bcore-worldgen/data/vanilla_worldgen.json")
    args = parser.parse_args()
    assets = {}
    worldgen = args.datapack / "data/minecraft/worldgen"
    for directory in ["density_function", "noise", "noise_settings"]:
        for path in sorted((worldgen / directory).rglob("*.json")):
            assets[path.relative_to(worldgen).as_posix()] = json.loads(path.read_text())
    assets["biome_parameters/overworld.json"] = json.loads(args.parameters.read_text())
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
    args.output.write_text(json.dumps(assets, separators=(",", ":"), sort_keys=True) + "\n", encoding="utf-8")
    print(f"Bundled {len(assets)} assets, {len(biomes)} biomes: {args.output.stat().st_size} bytes")
    print(f"sha256: {hashlib.sha256(args.output.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main()
