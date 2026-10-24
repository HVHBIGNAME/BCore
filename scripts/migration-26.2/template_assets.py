"""Verify independent spring catalogs and publish manager/data-fixed feature templates."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import native

CATALOG = "pool_alias_trial_combined_assets_26_2.json"
DESTINATION = "crates/bcore-worldgen/data/feature_template_assets_26_2.json"
BASIS = "crates/bcore-worldgen/data/pool_alias_trial_combined_assets_26_2.json"
FINGERPRINT = "1affd10b6938b00feda7fb9e22a00c5ff5b76a34446c6176059ade9ac22264ef"
TEMPLATES = {f"minecraft:spring/sulfur_spring_{suffix}" for suffix in
             ("small_1", "small_2", "small_3", "small_4", "medium_1", "medium_2", "medium_3",
              "large_1", "large_2", "extra_large_1")}


def verified(source: Path, pin: str) -> tuple[dict, dict]:
    if native.sha(source / "manifest.json") != pin:
        raise ValueError("spring catalog manifest changed")
    manifest = json.loads((source / "manifest.json").read_bytes())
    if (manifest["minecraft"], manifest["protocol"], manifest["jar_sha256"], manifest["mode"], manifest["families"]) != (
            "26.2", 776, native.JAR_SHA256, "jigsaw_assets", ["spring"]):
        raise ValueError("spring catalog identity/family differs")
    if manifest["authority"] != "native-catalog" or manifest["projection"] != "jigsaw":
        raise ValueError("spring catalog authority differs")
    for name, expected in manifest["files_sha256"].items():
        path = source / name
        if not path.resolve().is_relative_to(source) or native.sha(path) != expected:
            raise ValueError(f"spring catalog evidence changed: {name}")
    if native.sha(source / CATALOG) != manifest["catalog_sha256"]:
        raise ValueError("spring catalog bytes changed")
    return manifest, json.loads((source / CATALOG).read_bytes())


def project(catalog: dict, basis: dict, pins: list[str]) -> dict:
    for data in (catalog, basis):
        if (data["minecraft"], data["protocol"], data["jar_sha256"]) != ("26.2", 776, native.JAR_SHA256):
            raise ValueError("feature template registry identity differs")
    for key in ("states", "defaults", "water_fluid_id"):
        if catalog[key] != basis[key]:
            raise ValueError(f"spring and current structure registries differ: {key}")
    if set(catalog["templates"]) != TEMPLATES or set(catalog["template_sources"]) != TEMPLATES:
        raise ValueError("spring template inventory differs")
    for name, template in catalog["templates"].items():
        source = catalog["template_sources"][name]
        if source["missing_from_jar"] or source["path"] != f"data/minecraft/structure/{name.removeprefix('minecraft:')}.nbt":
            raise ValueError(f"template has no authenticated original source: {name}")
        size = template["size"]
        if len(size) != 3 or any(type(n) is not int or n < 1 for n in size) or template["data_version"] != 4903:
            raise ValueError(f"template dimensions/data version differ: {name}")
        palettes = template["palettes"]
        if not palettes:
            raise ValueError(f"template has no palette: {name}")
        for block in template["blocks"]:
            if len(block) not in (4, 5) or any(type(n) is not int for n in block[:4]):
                raise ValueError(f"invalid template block row: {name}")
            if any(not 0 <= p < extent for p, extent in zip(block[:3], size, strict=True)):
                raise ValueError(f"block outside native template dimensions: {name}")
            if any(not 0 <= block[3] < len(palette) for palette in palettes):
                raise ValueError(f"invalid native palette index: {name}")
    return {"schema": "bcore.native-feature-template-assets.v1", "minecraft": "26.2", "protocol": 776,
            "jar_sha256": native.JAR_SHA256, "registry_fingerprint": FINGERPRINT,
            "capture_manifest_sha256": pins, "templates": catalog["templates"],
            "template_sources": catalog["template_sources"], "block_entities": catalog["block_entities"]}


def publish(first: Path, first_pin: str, repeat: Path, repeat_pin: str, output: Path) -> None:
    native.environment()
    first_manifest, initial = verified(first, first_pin)
    repeat_manifest, repeated = verified(repeat, repeat_pin)
    if first == repeat or first_pin == repeat_pin:
        raise ValueError("feature templates require independent capture directories")
    if initial != repeated or first_manifest["source_sha256"] != repeat_manifest["source_sha256"]:
        raise ValueError("independent spring capture differs")
    basis = json.loads((native.ROOT / BASIS).read_bytes())
    result = project(repeated, basis, [first_pin, repeat_pin])
    output.mkdir()
    candidate = output / Path(DESTINATION).name
    native.write_json(candidate, result)
    destination = native.ROOT / DESTINATION
    expected = native.sha(candidate)
    if destination.exists():
        if native.sha(destination) != expected:
            raise ValueError("different feature template publication already exists")
    else:
        with destination.open("xb") as stream:
            stream.write(candidate.read_bytes())
    if native.sha(destination) != expected:
        raise ValueError("feature template publication readback differs")
    receipt = {"status": "pass", "destination": DESTINATION, "sha256": expected, "template_count": 10,
               "capture_manifest_sha256": [first_pin, repeat_pin], "catalog_sha256": native.sha(repeat / CATALOG),
               "basis_sha256": native.sha(native.ROOT / BASIS), "publisher_sha256": native.sha(Path(__file__))}
    native.write_json(output / "promotion.json", receipt)
    print(json.dumps(receipt, indent=2))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--first", required=True, type=Path)
    parser.add_argument("--first-manifest-sha256", required=True)
    parser.add_argument("--repeat", required=True, type=Path)
    parser.add_argument("--repeat-manifest-sha256", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists() or not output.parent.is_dir() or not output.is_relative_to(native.ROOT / "target"):
        parser.error("output must be absent with an existing parent under target/")
    publish(args.first.resolve(), args.first_manifest_sha256, args.repeat.resolve(), args.repeat_manifest_sha256, output)


if __name__ == "__main__":
    main()
