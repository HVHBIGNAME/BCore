"""Publish compiler-independent loot fixtures only after independent native equality."""

import argparse
import base64
from collections import Counter
import json
from pathlib import Path
import zipfile

from audit import compare, inventory_summary, load, observed_summary
from inventory import JAR_SHA256, ROOT, digest, new_output, write_json
from verify import require

REGISTRIES = ("items.json", "registry-types.json", "enchantment.json", "instrument.json", "potions.json", "loaded-tables.json")


def copy_file(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    raw = source.read_bytes()
    with destination.open("xb") as stream:
        stream.write(raw)
    require(digest(destination.read_bytes()) == digest(raw), f"copy differs: {destination}")


def stage(route, method):
    return next(item for item in route["stages"] if f".{method}(" in item["method"])


def fixture(case, source, context):
    route = case["direct"]
    generated = stage(route, "getRandomItems")
    slots = stage(route, "getAvailableSlots")
    split = stage(route, "shuffleAndSplitItems")
    spec = case["input"]
    result = {
        "id": spec["id"], "table": spec["table"], "seed_i64": spec["seed"],
        "container_size": int(spec["size"]), "context": context,
        "table_resolution": case.get("table_resolution", "registered"),
        "preexisting_slots": [item["stack"] if item else None for item in route["before"]],
        "expected_slots": [item["stack"] if item else None for item in route["after"]],
        "expected_slot_details": route["after"],
        "generated_stacks": generated["values"], "available_slots_shuffled": slots["values"],
        "shuffled_split_stacks": split["values"], "placement_order": route["placements"],
        "rng_after_generation": generated["rng"], "rng_after_slot_shuffle": slots["rng"],
        "rng_after_split_shuffle": split["rng"], "rng_after_fill": route["rng_after"],
        "rng_continuation": route["continuation"],
        "authority": {"capture": source.name, "case_id": spec["id"], "native_direct_fill": True,
                      "actual_chest_unpack_equal": "unpack" in case,
                      "first_trace_event": route["first_event"], "last_trace_event": route["last_event"]},
    }
    if "unpack" in case:
        unpack = case["unpack"]
        result["unpack"] = {"saved_before": unpack["saved_before"], "saved_after": unpack["saved_after"],
                            "loot_table_after": unpack["loot_table_after"], "loot_seed_after": unpack["loot_seed_after"],
                            "second_unpack_noop": unpack["second_unpack_slots"] == unpack["after"] and unpack["second_unpack_rng_unchanged"]}
    return result


def check_nbt(value):
    if isinstance(value, dict):
        if set(value) == {"base64", "sha256"}:
            raw = base64.b64decode(value["base64"], validate=True)
            require(digest(raw) == value["sha256"], "native NBT payload digest mismatch")
            require(raw[:3] == b"\x0a\x00\x00", "expected native unnamed compound NBT")
        for child in value.values():
            check_nbt(child)
    elif isinstance(value, list):
        for child in value:
            check_nbt(child)


def zip_bytes(archive, name, raw):
    info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    archive.writestr(info, raw)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pair", nargs=2, action="append", type=Path, required=True)
    parser.add_argument("--bytecode", action="append", type=Path, default=[])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    comparisons = [compare(first, repeat) for first, repeat in args.pair]
    out = new_output(args.output)
    primary = args.pair[0][0]
    summary = inventory_summary(primary)
    records, missing, inputs = [], [], []
    functions, patches = Counter(), Counter()
    native_fills = rng_checks = 0
    for first, repeat in args.pair:
        observations = load(first / "fixtures.json")
        verification = load(first / "verification.json")
        for name in REGISTRIES:
            require(load(first / name) == load(primary / name), f"registry context differs across matrices: {name}")
        context = {"world_seed_i64": observations["world_seed"], "dimension": "minecraft:overworld",
                   "block_pos": [int(v) for v in observations["context"]["origin_block"]],
                   "origin": [v + 0.5 for v in observations["context"]["origin_block"]],
                   "luck_f32_bits": "0", "player": None, "gameplay_ticks": observations["gameplay_ticks"]}
        for case in observations["cases"]:
            if case["status"] != "captured":
                missing.append({"capture": first.name, **case})
                continue
            value = fixture(case, first, context)
            check_nbt(value)
            records.append(value)
            for item in value["expected_slots"]:
                if item:
                    patches.update(item.get("components", {}).keys())
        functions.update(verification["function_calls"])
        native_fills += verification["native_fills"]
        rng_checks += verification["verified_rng_operations"]
        inputs.append({"first": first.name, "repeat": repeat.name,
                       "fixtures_sha256": digest((first / "fixtures.json").read_bytes()),
                       "provenance_sha256": digest((first / "provenance.json").read_bytes()),
                       "manifest_sha256": digest((first / "manifest.json").read_bytes()),
                       "repeat_manifest_sha256": digest((repeat / "manifest.json").read_bytes())})
        write_json(out / (first.name + "-case-summary.json"), observed_summary(first))
    require(len({case["id"] for case in records}) == len(records), "duplicate fixture case IDs")
    data = {"schema": "bcore.native-loot-fill.v1", "minecraft": "26.1", "protocol": 775,
            "jar_sha256": JAR_SHA256, "authority": inputs,
            "contract": {"slot_arrays": "Index is container slot; null is empty. ItemStack JSON carries the native component patch.",
                         "effective_components": "expected_slot_details contains the full native persistent component map plus NBT bytes.",
                         "rng": "Explicit nonzero seed => LegacyRandomSource. All i64/state/IEEE-bit values use decimal strings.",
                         "scope": "Finite actual-engine fills with luck=0 and no player; seed0 is not captured."},
            "cases": records}
    write_json(out / "loot-fill-26.1.json", data)
    inventory = load(primary / "table-inventory.json")
    tables = {}
    for table in inventory["tables"]:
        raw = (primary / "jar-resources" / table["jar_member"]).read_bytes()
        require(digest(raw) == table["sha256"], "raw table identity differs")
        tables[table["key"]] = json.loads(raw)
    write_json(out / "tables.json", tables)
    for name in (*REGISTRIES, "table-inventory.json", "native-classes.json"):
        copy_file(primary / name, out / name)
    for name in ("ALGORITHMS.md", "README.md", "package.py", "audit.py", "inventory.py", "verify.py"):
        copy_file(Path(__file__).parent / name, out / name)
    pending_world = [entry for entry in summary["chest_or_trial_candidates"] if entry["context_sensitive_candidates"]]
    coverage = {"status": "pass", "fixture_cases": len(records), "native_fills_per_independent_pass": native_fills,
                "verified_rng_operations_per_pass": rng_checks, "independent_passes": 2,
                "registered_candidate_tables_evaluated": len({c["table"] for c in records if c["table_resolution"] == "registered"}),
                "all_jar_loot_tables": summary["all_loot_tables"], "chest_or_trial_candidates": len(summary["chest_or_trial_candidates"]),
                "item_registry_count": summary["item_count"], "item_tag_count": summary["item_tag_count"],
                "enchantment_count": summary["enchantment_count"], "potion_count": summary["potion_count"],
                "instrument_count": summary["instrument_count"], "function_calls_per_pass": dict(functions),
                "component_patch_counts_in_expected_slots": dict(patches), "pending_world_tables": pending_world,
                "initial_missing_lookup_cases": missing,
                "empty_table_resolution": "minecraft:empty is not registered in the loaded built-in tables; additional fixtures exercise native EMPTY fallback explicitly.",
                "uncovered": ["Seed0 named sequences and level RNG, including persistence/reload", "Real player/nonzero luck/advancement/spectator branches",
                              "World-dependent exploration maps and their set_name execution", "Double-chest and non-chest container lifecycles",
                              "Arbitrary custom datapack functions/conditions and cycles", "Disabled feature items and arbitrary over-max stacks",
                              "Transient component values/removal patches beyond the recorded inputs", "Performance relative to uninstrumented Java"]}
    write_json(out / "coverage.json", coverage)
    write_json(out / "independent-repeats.json", comparisons)
    write_json(out / "inventory-summary.json", summary)
    with zipfile.ZipFile(out / "evidence.zip", "x") as archive:
        for pair in args.pair:
            for capture in pair:
                for path in sorted(capture.rglob("*")):
                    if not path.is_file():
                        continue
                    relative = path.relative_to(capture)
                    if relative.parts[0] in ("jar-resources", "server"):
                        continue
                    zip_bytes(archive, capture.name + "/" + relative.as_posix(), path.read_bytes())
                for name in ("server.properties", "eula.txt"):
                    path = capture / "server" / name
                    zip_bytes(archive, capture.name + "/server/" + name, path.read_bytes())
        for directory in args.bytecode:
            provenance = load(directory / "provenance.json")
            require(provenance["jar_sha256"] == JAR_SHA256, "disassembly uses a different native JAR")
            for entry in provenance["classes"]:
                require(digest((directory / entry["output"]).read_bytes()) == entry["output_sha256"], "disassembly bytes changed")
            for path in sorted(directory.iterdir()):
                if path.is_file():
                    zip_bytes(archive, directory.name + "/" + path.name, path.read_bytes())
        for path in sorted((primary / "jar-resources").rglob("*.json")):
            zip_bytes(archive, "jar-resources/" + path.relative_to(primary / "jar-resources").as_posix(), path.read_bytes())
        for path in sorted(out.iterdir()):
            if path.is_file() and path.name != "evidence.zip":
                zip_bytes(archive, "integration/" + path.name, path.read_bytes())
    manifest = {p.name: digest(p.read_bytes()) for p in sorted(out.iterdir()) if p.is_file()}
    write_json(out / "manifest.json", manifest)
    with zipfile.ZipFile(out / "evidence.zip") as archive:
        require(archive.testzip() is None, "packaged evidence ZIP failed CRC validation")
    require(load(out / "loot-fill-26.1.json") == data, "portable fixture JSON readback differs")
    print(json.dumps({"output": str(out), "fixture_cases": len(records), "native_fills_per_pass": native_fills,
                      "fixtures_sha256": manifest["loot-fill-26.1.json"], "evidence_sha256": manifest["evidence.zip"],
                      "status": "pass"}, indent=2))


if __name__ == "__main__":
    main()
