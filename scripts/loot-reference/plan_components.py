"""Choose a tiny explicit native trial matrix using the already-verified LCG contract.

Predicted entry selection only chooses inputs; all item/component results still
come from the original server engine. No loot table is synthesized or modified.
"""

import argparse
import json
from pathlib import Path

from inventory import digest, new_output, write_json
from verify import MASK, MULTIPLIER, Rng, require


def selection(table, seed):
    require(len(table["pools"]) == 1, "planner only supports one native pool")
    pool = table["pools"][0]
    require(pool["rolls"] == 1 and pool["bonus_rolls"] == 0 and not pool.get("conditions"), "unexpected RNG prefix")
    require(all(entry["type"] == "minecraft:item" and not entry.get("conditions") for entry in pool["entries"]), "unexpected native entry type")
    value = Rng((seed ^ MULTIPLIER) & MASK).integer(sum(entry.get("weight", 1) for entry in pool["entries"]))
    for entry in pool["entries"]:
        value -= entry.get("weight", 1)
        if value < 0:
            return entry
    raise ValueError("unreachable weighted entry")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    out = new_output(args.output)
    original = json.loads((args.capture / "build/cases.json").read_text(encoding="utf-8"))
    targets = (
        ("reward_common", "minecraft:set_ominous_bottle_amplifier", "SetOminousBottleAmplifierFunction"),
        ("reward_ominous_rare", "minecraft:set_enchantments", "SetEnchantmentsFunction"),
    )
    cases, provenance = [], []
    for name, function, java_class in targets:
        path = args.capture / f"jar-resources/data/minecraft/loot_table/chests/trial_chambers/{name}.json"
        table = json.loads(path.read_text(encoding="utf-8"))
        for sign in (1, -1):
            selected = next((sign * n for n in range(2, 129)
                             if any(f["function"] == function for f in selection(table, sign * n).get("functions", []))), None)
            require(selected is not None, "no planned seed in the bounded 2..128 search")
            cases.append({"id": name + ("-positive" if sign == 1 else "-negative"),
                          "table": "minecraft:chests/trial_chambers/" + name, "seed": str(selected), "size": 27,
                          "preexisting": [], "expected_function_class": "world/level/storage/loot/functions/" + java_class})
            provenance.append({"source": str(path.resolve()), "sha256": digest(path.read_bytes()),
                               "seed": str(selected), "prediction_only": selection(table, selected), "searched_abs_seed": [2, 128]})
    for name, size, full in (("empty-fallback", 27, False), ("empty-fallback-full", 27, True), ("empty-fallback-zero-size", 0, False)):
        row = {"id": name, "table": "minecraft:empty", "seed": "-19", "size": size, "preexisting": [],
               "missing_table_policy": "native_lookup"}
        if full:
            row["fill_existing"] = {"id": "minecraft:stone", "count": 64}
        cases.append(row)
    cases.append({"id": "unregistered-fallback", "table": "loot_reference:absent", "seed": "29", "size": 27,
                  "preexisting": [{"slot": 5, "stack": {"id": "minecraft:diamond", "count": 7}}],
                  "missing_table_policy": "native_lookup"})
    write_json(out / "cases-components.json", {**original, "cases": cases})
    write_json(out / "planning-authority.json", {"status": "input selection only; native results not yet observed",
               "original_tables": provenance, "empty_table": "Explicitly exercise native ReloadableServerRegistries.Holder.getLootTable fallback; no synthetic replacement"})
    for filename in ("plan_components.py", "inventory.py", "verify.py"):
        with (out / filename).open("xb") as stream:
            stream.write((Path(__file__).parent / filename).read_bytes())
    print(json.dumps({"output": str(out), "cases": len(cases), "seeds": [case["seed"] for case in cases]}))


if __name__ == "__main__":
    main()
