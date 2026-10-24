"""Promote pinned native 26.1 loot inputs into this repo or a linked worktree.

Copies retain their original bytes. Existing different files are refused. The
runtime catalog and test oracle have separate destinations; no JVM is required.
Run through run_generation_check.py, selecting --workdir for a linked worktree.
"""
import argparse
import hashlib
import json
from pathlib import Path


PINNED = {
    "tables.json": "de0a11dbe5312eaf19e6ea26ae213e02287e8ab8e999215de6893d869c17a847",
    "items.json": "aa14a99f3525ec1de6d17f749c813512484dba738739f40f8eefc2ccc184dfac",
    "enchantment.json": "99221f9ea6ffb2db0200d9491f7c6ad601ef756e76dbbf40e2e0fde34d39a680",
    "potions.json": "65268536e1db3619439a9293d95fbf837d49d99883ecf47b923165e4d0a3fc12",
    "instrument.json": "0aa54ea01b410ec7df48bc4c74f74bb37599d305cad06edb8160c44304c48667",
    "registry-types.json": "4621857941eab22239469fd5c294d5b348725a95930d9c8b63f3b7125c26ac71",
    "loot-fill-26.1.json": "2158fd855f1bbc46d6b399620c9ced35822263c982151686f58e175c1ccf9db1",
}


def publish(path, contents):
    if path.exists():
        if path.read_bytes() != contents:
            raise ValueError(f"refusing to replace different data: {path}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as output:
        output.write(contents)
    if path.read_bytes() != contents:
        raise ValueError(f"promotion readback mismatch: {path}")


def inspect_catalog(data):
    tables = data["tables.json"]
    candidates = {k: v for k, v in tables.items()
                  if v.get("type") == "minecraft:chest" or "trial_chambers" in k}
    functions, conditions, entries, providers = {}, {}, set(), set()

    def walk(value):
        if isinstance(value, list):
            for child in value:
                walk(child)
        elif isinstance(value, dict):
            if "function" in value:
                family = value["function"]
                functions.setdefault(family, {})[json.dumps(value, sort_keys=True)] = value
            if "condition" in value:
                conditions.setdefault(value["condition"], value)
            if "type" in value:
                if "entries" in value or "name" in value or "value" in value:
                    entries.add(value["type"])
                if "min" in value or "max" in value:
                    providers.add(value["type"])
            for child in value.values():
                walk(child)

    for table in candidates.values():
        walk(table)
    print("Candidate tables:", len(candidates), "entry/provider types:", sorted(entries), sorted(providers))
    print("Conditions:", json.dumps(conditions, sort_keys=True))
    for name, variants in functions.items():
        print("FUNCTION", name, "variants", len(variants))
        if name != "minecraft:set_count":
            for variant in variants.values():
                print(json.dumps(variant, sort_keys=True))
    for name in ("items.json", "enchantment.json", "potions.json", "instrument.json", "registry-types.json"):
        print("SCHEMA", name, list(data[name]) if isinstance(data[name], dict) else type(data[name]).__name__)
    for item in data["items.json"]["items_native_order"]:
        if item["key"] in {"minecraft:book", "minecraft:enchanted_book", "minecraft:iron_pickaxe",
                           "minecraft:suspicious_stew", "minecraft:potion", "minecraft:ominous_bottle"}:
            print("ITEM", json.dumps(item, sort_keys=True))
    for value in data["enchantment.json"]["values_native_order"]:
        print("ENCHANTMENT", value["key"], json.dumps({k: v for k, v in value["value"].items()
              if k not in {"effects", "description", "slots"}}, sort_keys=True))
    for tag in data["enchantment.json"]["tags"]:
        print("ENCHANTMENT_TAG", json.dumps(tag, sort_keys=True))
    print("INSTRUMENTS", json.dumps(data["instrument.json"], sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--worktree", type=Path, required=True)
    parser.add_argument("--inspect", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    worktree = args.worktree.resolve()
    if worktree == root:
        if not (root / ".git").exists():
            parser.error("repository root must have Git metadata")
    else:
        if not worktree.is_relative_to(root / "target"):
            parser.error("destination must be this repository root or a linked worktree under target")
        if not (worktree / ".git").is_file():
            parser.error("destination must be an existing linked worktree")
    source = args.source.resolve()
    manifest = json.loads((source / "manifest.json").read_bytes())
    payloads, data = {}, {}
    for name, expected in PINNED.items():
        contents = (source / name).read_bytes()
        digest = hashlib.sha256(contents).hexdigest()
        if digest != expected or manifest.get(name) != expected:
            raise ValueError(f"native input hash mismatch: {name}")
        payloads[name] = contents
        data[name] = json.loads(contents)
    fixture = data["loot-fill-26.1.json"]
    if fixture["schema"] != "bcore.native-loot-fill.v1" or len(fixture["cases"]) != 137:
        raise ValueError("unexpected native fixture schema or coverage")
    crate = worktree / "crates/bcore-worldgen"
    outputs = {}
    for name, contents in payloads.items():
        base = crate / ("tests/data/loot_26_1" if name == "loot-fill-26.1.json" else "data/loot_26_1")
        destination = base / name
        publish(destination, contents)
        outputs[destination.relative_to(worktree).as_posix()] = {
            "source": name, "bytes": len(contents), "sha256": PINNED[name]}
    provenance = {"schema": "bcore.loot-data-promotion.v1", "minecraft": "26.1",
                  "server_sha256": "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52",
                  "files": outputs}
    publish(crate / "data/loot_26_1/promotion.json",
            (json.dumps(provenance, indent=2, sort_keys=True) + "\n").encode())
    print(json.dumps(provenance, indent=2, sort_keys=True))
    if args.inspect:
        inspect_catalog(data)


if __name__ == "__main__":
    main()
