"""Export immutable 26.1 loot resources and class identities without executing them."""

import argparse
import hashlib
import json
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[2]
JAR = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
RESOURCE_PREFIXES = (
    "data/minecraft/loot_table/", "data/minecraft/enchantment/",
    "data/minecraft/tags/enchantment/", "data/minecraft/tags/item/",
    "data/minecraft/tags/instrument/", "data/minecraft/instrument/",
    "data/minecraft/jukebox_song/", "data/minecraft/trim_pattern/",
    "data/minecraft/trim_material/",
)
CLASS_PREFIXES = (
    "net/minecraft/world/level/storage/loot/", "net/minecraft/world/item/",
    "net/minecraft/core/component/", "net/minecraft/world/RandomSequence",
    "net/minecraft/world/RandomizableContainer", "net/minecraft/util/RandomSource",
    "net/minecraft/world/level/levelgen/LegacyRandomSource",
    "net/minecraft/world/level/levelgen/BitRandomSource",
    "net/minecraft/world/level/levelgen/Xoroshiro", "net/minecraft/world/level/levelgen/RandomSupport",
    "net/minecraft/world/level/block/entity/RandomizableContainerBlockEntity",
    "net/minecraft/world/level/block/entity/BaseContainerBlockEntity",
    "net/minecraft/world/level/block/entity/ChestBlockEntity",
)
WORLD_FUNCTIONS = {
    "minecraft:exploration_map", "minecraft:copy_custom_data", "minecraft:copy_components",
    "minecraft:copy_name", "minecraft:fill_player_head", "minecraft:enchanted_count_increase",
    "minecraft:looting_enchant", "minecraft:apply_bonus", "minecraft:furnace_smelt",
}
WORLD_CONDITIONS = {
    "minecraft:location_check", "minecraft:entity_properties", "minecraft:entity_scores",
    "minecraft:weather_check", "minecraft:time_check", "minecraft:killed_by_player",
    "minecraft:match_tool", "minecraft:survives_explosion", "minecraft:damage_source_properties",
    "minecraft:random_chance_with_enchanted_bonus", "minecraft:enchantment_active_check",
}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write_json(path, value):
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False, sort_keys=True)
        stream.write("\n")


def new_output(path):
    path = path.resolve()
    if path.parent != ROOT / "target" or not path.name.startswith("loot-native-20261016-"):
        raise ValueError("output must be target/loot-native-20261016-* in this repository")
    path.mkdir(exist_ok=False)
    return path


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def table_facts(name, value, raw):
    objects = list(nodes(value))
    functions = sorted({n["function"] for n in objects if isinstance(n.get("function"), str)})
    conditions = sorted({n["condition"] for n in objects if isinstance(n.get("condition"), str)})
    entries = sorted({n["type"] for n in objects if n.get("type", "").startswith("minecraft:")
                      and ("name" in n or "children" in n or n["type"] in
                           ("minecraft:empty", "minecraft:loot_table", "minecraft:dynamic", "minecraft:tag"))})
    references = sorted({n.get("value", n.get("name")) for n in objects
                         if n.get("type") == "minecraft:loot_table"
                         and isinstance(n.get("value", n.get("name")), str)})
    return {
        "key": "minecraft:" + name.removeprefix("data/minecraft/loot_table/").removesuffix(".json"),
        "jar_member": name, "sha256": digest(raw), "bytes": len(raw),
        "type": value.get("type"), "random_sequence": value.get("random_sequence"),
        "functions": functions, "conditions": conditions, "entry_types": entries,
        "referenced_tables": references,
        "item_keys": sorted({n["name"] for n in objects if n.get("type") == "minecraft:item"}),
        "context_sensitive_candidates": sorted(set(functions) & WORLD_FUNCTIONS | set(conditions) & WORLD_CONDITIONS),
    }


def export(output):
    jar_bytes = JAR.read_bytes()
    if digest(jar_bytes) != JAR_SHA256:
        raise ValueError("pinned 26.1 server JAR SHA-256 mismatch")
    resources, tables, classes = [], [], []
    extracted_bytes = 0
    with zipfile.ZipFile(JAR) as archive:
        for entry in sorted(archive.infolist(), key=lambda item: item.filename):
            name = entry.filename
            if name.endswith(".json") and name.startswith(RESOURCE_PREFIXES):
                if entry.file_size > 2**22 or extracted_bytes + entry.file_size > 2**26 or len(resources) >= 8192:
                    raise ValueError("loot resource inventory exceeds the bounded export budget")
                raw = archive.read(entry)
                extracted_bytes += len(raw)
                path = output / "jar-resources" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                with path.open("xb") as stream:
                    stream.write(raw)
                resources.append({"member": name, "sha256": digest(raw), "bytes": len(raw)})
                if name.startswith("data/minecraft/loot_table/"):
                    tables.append(table_facts(name, json.loads(raw), raw))
            if name.endswith(".class") and name.startswith(CLASS_PREFIXES):
                raw = archive.read(entry)
                classes.append({"member": name, "sha256": digest(raw), "bytes": len(raw),
                                "major_version": int.from_bytes(raw[6:8], "big")})
    for record in tables:
        record["chest_or_trial_candidate"] = record["key"].startswith("minecraft:chests/") or "trial_chambers/" in record["key"]
    inventory = {"schema": 1, "jar_sha256": JAR_SHA256, "tables": tables, "resources": resources,
                 "resource_count": len(resources), "table_count": len(tables),
                 "chest_or_trial_count": sum(t["chest_or_trial_candidate"] for t in tables),
                 "extracted_bytes": extracted_bytes,
                 "coverage": "All data/minecraft/loot_table/*.json members including subdirectories; selected registry/tag resources.",
                 "context_classification": "Static candidates only. Absence from candidate lists does not prove context independence."}
    write_json(output / "table-inventory.json", inventory)
    write_json(output / "native-classes.json", {"jar_sha256": JAR_SHA256, "classes": classes})
    return inventory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = new_output(args.output)
    data = export(out)
    with (out / "inventory.py").open("xb") as stream:
        stream.write(Path(__file__).read_bytes())
    write_json(out / "provenance.json", {"jar_sha256": JAR_SHA256,
               "runner_sha256": digest(Path(__file__).read_bytes()),
               "table_inventory_sha256": digest((out / "table-inventory.json").read_bytes()),
               "native_classes_sha256": digest((out / "native-classes.json").read_bytes())})
    print(json.dumps({"output": str(out), "tables": data["table_count"],
                      "chest_or_trial_candidates": data["chest_or_trial_count"]}))


if __name__ == "__main__":
    main()
