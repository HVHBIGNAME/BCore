"""Audit immutable native captures and plan the next bounded built-in-table matrix."""

import argparse
from collections import Counter
import itertools
import json
from pathlib import Path

from inventory import digest, new_output, write_json
from verify import canonical, normalize_trace, require

COMPARE_FILES = ("fixtures.json", "items.json", "registry-types.json", "enchantment.json",
                 "instrument.json", "potions.json", "loaded-tables.json")


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def verify_manifest(path):
    manifest = load(path / "manifest.json")
    for name, expected in manifest.items():
        require(digest((path / name).read_bytes()) == expected, f"capture changed: {path / name}")
    return len(manifest)


def compare(left, right):
    manifests = {str(path): verify_manifest(path) for path in (left, right)}
    a, b = load(left / "provenance.json"), load(right / "provenance.json")
    for key in ("jar_sha256", "asm_sha256", "java_executable_sha256", "javac_executable_sha256",
                "sources", "cases_sha256", "compiled_classes", "libraries", "server_properties"):
        require(a[key] == b[key], f"capture authority differs: {key}")
    equality = {}
    for name in COMPARE_FILES:
        a, b = load(left / name), load(right / name)
        require(a == b, f"independent native repeat differs: {name}")
        equality[name] = {"equal": True, "canonical_sha256": digest(canonical(a).encode())}
    count = 0
    with (left / "trace.jsonl").open(encoding="utf-8") as a, (right / "trace.jsonl").open(encoding="utf-8") as b:
        for count, (first, second) in enumerate(itertools.zip_longest(a, b), 1):
            require(first is not None and second is not None, "repeat trace lengths differ")
            require(normalize_trace(json.loads(first)) == normalize_trace(json.loads(second)), f"native trace differs at event {count}")
    return {"status": "pass", "left": str(left.resolve()), "right": str(right.resolve()),
            "verified_manifest_files": manifests, "equal_files": equality, "equal_trace_events": count,
            "trace_normalization": "Only class metadata matching $$Lambda/0x<hex> has the runtime address replaced. All event/slot/item/component/RNG fields and ordering are exact."}


def inventory_summary(capture):
    inventory = load(capture / "table-inventory.json")
    candidates = [t for t in inventory["tables"] if t["chest_or_trial_candidate"]]
    tables = {t["key"]: t for t in inventory["tables"]}

    def closure(key, visited=None):
        visited = set() if visited is None else visited
        if key in visited:
            return visited
        visited.add(key)
        for other in tables[key]["referenced_tables"]:
            require(other in tables, f"missing table dependency {other}")
            closure(other, visited)
        return visited

    function_tables = {}
    candidates_info = []
    for table in candidates:
        dependencies = closure(table["key"])
        functions = sorted({f for name in dependencies for f in tables[name]["functions"]})
        conditions = sorted({c for name in dependencies for c in tables[name]["conditions"]})
        sensitive = sorted({f for name in dependencies for f in tables[name]["context_sensitive_candidates"]})
        for function in functions:
            function_tables.setdefault(function, []).append(table["key"])
        candidates_info.append({"key": table["key"], "type": table["type"], "functions_including_dependencies": functions,
                                "conditions_including_dependencies": conditions, "dependencies": sorted(dependencies - {table["key"]}),
                                "context_sensitive_candidates": sensitive})
    items = load(capture / "items.json")
    return {"jar_sha256": inventory["jar_sha256"], "all_loot_tables": inventory["table_count"],
            "chest_or_trial_candidates": candidates_info, "function_tables": function_tables,
            "item_count": len(items["items_native_order"]), "item_tag_count": len(items["tags"]),
            "enchantment_count": len(load(capture / "enchantment.json")["values_native_order"]),
            "instrument_count": len(load(capture / "instrument.json")["values_native_order"]),
            "potion_count": len(load(capture / "potions.json"))}


def plan(capture, summary):
    original = load(capture / "build/cases.json")
    cases, skipped = [], []
    for index, table in enumerate(summary["chest_or_trial_candidates"]):
        if table["context_sensitive_candidates"]:
            skipped.append({"table": table["key"], "reason": "world-dependent function requires a separately bounded native location/persistent-world capture",
                            "functions": table["context_sensitive_candidates"]})
            continue
        for seed in ("1", "-1"):
            cases.append({"id": f"table-{index:02d}-{'positive' if seed == '1' else 'negative'}",
                          "table": table["key"], "seed": seed, "size": 27, "preexisting": []})
    # Seed endpoints and preoccupied slots remain explicit cases, not a seed search.
    cases.extend(original["cases"])
    cases.extend([
        {"id": "empty-table", "table": "minecraft:empty", "seed": "1", "size": 27, "preexisting": []},
        {"id": "empty-table-no-slots", "table": "minecraft:empty", "seed": "-1", "size": 0, "preexisting": []},
        {"id": "one-empty-slot", "table": "minecraft:chests/simple_dungeon", "seed": "1", "size": 27,
         "fill_existing": {"id": "minecraft:stone", "count": 64}, "preexisting": [{"slot": 26, "stack": None}]},
        {"id": "same-item-preexisting", "table": "minecraft:chests/simple_dungeon", "seed": "1", "size": 27,
         "preexisting": [{"slot": 0, "stack": {"id": "minecraft:gunpowder", "count": 63}}]},
        {"id": "component-preexisting", "table": "minecraft:chests/ancient_city", "seed": "123456789", "size": 27,
         "preexisting": [{"slot": 7, "stack": {"id": "minecraft:potion", "count": 1,
            "components": {"minecraft:potion_contents": {"potion": "minecraft:strong_healing"}}}}]},
    ])
    return {**original, "cases": cases}, skipped


def observed_summary(capture):
    data = load(capture / "fixtures.json")
    rows, patches = [], Counter()
    for case in data["cases"]:
        if case["status"] != "captured":
            rows.append({"id": case["input"]["id"], "status": case["status"]})
            continue
        route = case["direct"]
        items = [{"slot": i, **item["stack"]} for i, item in enumerate(route["after"]) if item]
        for item in items:
            patches.update(item.get("components", {}).keys())
        rows.append({"id": case["input"]["id"], "table": case["input"]["table"], "seed": case["input"]["seed"],
                     "size": case["input"]["size"], "rng_after": route["rng_after"], "slots": items})
    return {"cases": rows, "observed_component_patch_counts": dict(patches)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--repeat", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--plan", action="store_true")
    args = parser.parse_args()
    out = new_output(args.output)
    for name in ("audit.py", "inventory.py", "verify.py"):
        with (out / name).open("xb") as stream:
            stream.write((Path(__file__).parent / name).read_bytes())
    summary = inventory_summary(args.capture)
    write_json(out / "inventory-summary.json", summary)
    write_json(out / "case-summary.json", observed_summary(args.capture))
    if args.repeat:
        result = compare(args.capture, args.repeat)
        write_json(out / "independent-repeat.json", result)
    if args.plan:
        cases, skipped = plan(args.capture, summary)
        write_json(out / "cases-expanded.json", cases)
        write_json(out / "world-dependent-pending.json", skipped)
    print(json.dumps({"output": str(out), "item_count": summary["item_count"],
                      "enchantment_count": summary["enchantment_count"], "chest_or_trial_count": len(summary["chest_or_trial_candidates"]),
                      "independent_repeat": bool(args.repeat), "expanded_case_count": len(cases["cases"]) if args.plan else None}))


if __name__ == "__main__":
    main()
