"""Verify saved/load chest evidence and project compact typed-NBT fixtures."""

import base64
import json

from inventory import JAR_SHA256, digest, write_json


def require(condition, message):
    if not condition:
        raise ValueError(message)


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def blob_value(blob, codec):
    raw = base64.b64decode(blob["base64"], validate=True)
    require(digest(raw) == blob["sha256"], "NBT SHA differs from the native bytes")
    return {"base64": blob["base64"], "typed": codec.NbtReader(raw).root()}


def nbt_slots(root, codec):
    result = [None] * 27
    for item in root.get("Items", {}).get("List", {}).get("values", []):
        fields = item["Compound"]
        require(set(fields["Slot"]) == {"Byte"}, "native saved Slot must be a Byte")
        slot = fields["Slot"]["Byte"]
        require(0 <= slot < 27, "unexpected out-of-range fixture slot")
        result[slot] = {name: codec.plain(value) for name, value in fields.items() if name != "Slot"}
    return result


def memory_slots(snapshot):
    return [item["stack"] if item else None for item in snapshot["memory"]["slots"]]


def method_count(operation, prefix):
    return sum(count for name, count in operation["method_entries"].items() if name.startswith(prefix))


def random_entries(operation):
    return sum(count for name, count in operation["method_entries"].items()
               if "/levelgen/" in name or name.startswith("util/RandomSource.") or ".getRandomSequence(" in name)


def check_silent(operation):
    require(not operation["fill_seeds_i64"] and not operation["new_contexts"], "no-op/load/save created new loot context")
    require(random_entries(operation) == 0, "no-op/load/save called native RNG methods")
    require(operation["loot_rng_states_before"] == operation["loot_rng_states_after"], "no-op/load/save advanced prior loot RNG")
    require(method_count(operation, "world/level/storage/loot/LootTable.fill(") == 0, "no-op/load/save rerolled loot")


def verify_case(case, spec, codec):
    snapshots = {snapshot["boundary"]: snapshot for snapshot in case["snapshots"]}
    operations = {operation["operation"]: operation for operation in case["operations"]}
    require(len(snapshots) == 6 and len(operations) == 12, "incomplete lifecycle boundary capture")
    roots = {name: blob_value(snapshot["saved"], codec)["typed"]["Compound"] for name, snapshot in snapshots.items()}
    source_root = blob_value(case["input"], codec)["typed"]["Compound"]
    loaded = snapshots["loaded_before_unpack"]
    first, second = snapshots["after_first_unpack"], snapshots["after_second_unpack"]
    fresh = snapshots["fresh_loaded_materialized"]
    expected_table, expected_seed = spec["expected_table"], spec["expected_seed_i64"]
    require(loaded["memory"]["loot_table"] == expected_table and loaded["memory"]["loot_seed_i64"] == expected_seed,
            f"wrong initial table/seed: {case['id']}")
    for snapshot in snapshots.values():
        require(snapshot["memory"]["native_level_attached"] and len(snapshot["memory"]["slots"]) == 27,
                "chest not attached to native ServerLevel or wrong size")
    if expected_table is not None:
        require(all(item is None for item in memory_slots(loaded)), "deferred native load retained Items")
        require(roots["loaded_before_unpack"]["LootTable"] == {"String": expected_table}, "saved deferred key differs")
        require(roots["loaded_before_unpack"]["LootTableSeed"] == {"Long": int(expected_seed)}, "saved deferred seed differs")
        require("Items" not in roots["loaded_before_unpack"], "saved deferred tag retains backing Items")
        require(method_count(operations["load_initial"], "world/ContainerHelper.loadAllItems(") == 0,
                "native load took Items branch while a valid table exists")
        roll = operations["first_unpack_null"]
        require(roll["fill_seeds_i64"] == [expected_seed] and len(roll["new_contexts"]) == 1, "first unpack did not fill exactly once")
        require(roll["table_resolutions"] == [{"is_native_empty": spec["expected_native_empty"]}], "wrong native table lookup")
        for context in roll["new_contexts"]:
            require(context["same_server_level"] and context["this_entity"] is None and context["luck_f32_bits"] == "0",
                    "unexpected loot context")
            require(context["origin"] == [8.5, 80.5, 8.5], "unpack origin is not the block center")
        if spec["expected_native_empty"]:
            require(all(item is None for item in memory_slots(first)), "missing key did not produce native empty loot")
        if "prior_after_slots" in spec:
            require(memory_slots(first) == spec["prior_after_slots"], "loaded deferred fill differs from previous independent native authority")
            previous = blob_value(spec["prior_after_nbt"], codec)["typed"]["Compound"]
            require(roots["after_first_unpack"] == previous, "materialized saved NBT differs from prior native capture")
    else:
        require(memory_slots(loaded) == nbt_slots(source_root, codec), "preset Items failed to load")
        require(method_count(operations["load_initial"], "world/ContainerHelper.loadAllItems(") == 1, "materialized load skipped Items")
        check_silent(operations["first_unpack_null"])
        require(memory_slots(first) == memory_slots(loaded), "materialized first unpack changed preset Items")
    if spec["input_kind"] == "deferred_plus_items":
        require(sum(item is not None for item in nbt_slots(source_root, codec)) == 3, "missing conflicting input Items")
    require(first["memory"]["loot_seed_i64"] == expected_seed and first["memory"]["loot_table"] is None,
            "first unpack did not retain seed/clear key")
    require(first["memory"] == second["memory"] and roots["after_first_unpack"] == roots["after_second_unpack"], "second unpack changed chest")
    require(fresh["memory"]["loot_seed_i64"] == "0" and fresh["memory"]["loot_table"] is None, "fresh materialized load did not default seed to zero")
    require(memory_slots(fresh) == memory_slots(first), "fresh load changed materialized slot components")
    require(method_count(operations["load_saved_materialized_into_fresh_chest"], "world/level/block/entity/BlockEntity.loadStatic(") == 1,
            "fresh reopen did not use native loadStatic")
    require(method_count(operations["load_saved_materialized_into_fresh_chest"], "world/ContainerHelper.loadAllItems(") == 1,
            "fresh materialized load did not load Items")
    for name in ("after_first_unpack", "after_second_unpack", "fresh_loaded_materialized", "after_reopened_unpack", "after_reopened_getItem"):
        root = roots[name]
        require("LootTable" not in root and "LootTableSeed" not in root and "Items" in root, "wrong materialized saved shape")
        require(nbt_slots(root, codec) == memory_slots(snapshots[name]), "saved typed NBT slots differ from live component-bearing stacks")
        require(root == roots["after_first_unpack"], "materialized load/reopen changed saved NBT")
    for name in ("after_reopened_unpack", "after_reopened_getItem"):
        require(snapshots[name]["memory"] == fresh["memory"], "reopen changed materialized in-memory chest")
    for name, operation in operations.items():
        if name != "first_unpack_null":
            check_silent(operation)
    return {"id": case["id"], "status": "pass", "first_fill_count": len(operations["first_unpack_null"]["fill_seeds_i64"]),
            "materialized_occupied_slots": sum(item is not None for item in memory_slots(first)),
            "initial_seed_i64": expected_seed, "fresh_materialized_seed_i64": "0", "reopen_rng_entries": 0,
            "table_over_items": spec["input_kind"] == "deferred_plus_items", "saved_slot_type": "Byte"}


def compact(raw, config, codec):
    blobs, components, cases = {}, {}, []

    def intern(blob):
        decoded = blob_value(blob, codec)
        key = blob["sha256"]
        if key in blobs:
            require(blobs[key] == decoded, "NBT digest collision")
        blobs[key] = decoded
        return key

    for source in raw["cases"]:
        snapshots = {}
        for snapshot in source["snapshots"]:
            memory = snapshot["memory"]
            for item in memory["slots"]:
                if item:
                    key = digest(canonical(item["stack"]).encode())
                    require(key not in components or components[key] == item, "same serialized stack had different persistent components")
                    components[key] = item
            snapshots[snapshot["boundary"]] = {"loot_table": memory["loot_table"], "loot_seed_i64": memory["loot_seed_i64"],
                "slots": memory_slots(snapshot), "saved_nbt_sha256": intern(snapshot["saved"]),
                "native_level_attached": memory["native_level_attached"]}
        cases.append({"id": source["id"], "input_nbt_sha256": intern(source["input"]),
                      "fresh_load_api": source["fresh_load_api"], "snapshots": snapshots, "operations": source["operations"]})
    return {"schema": "bcore.native-chest-lifecycle.v1", "minecraft": "26.1", "protocol": 775, "jar_sha256": JAR_SHA256,
            "inputs_sha256": digest(canonical(config).encode()), "world_seed_i64": raw["world_seed_i64"],
            "block_pos": [int(v) for v in raw["block_pos"]], "player": None, "menu_creation_exercised": False,
            "gameplay_ticks": raw["gameplay_ticks"], "nbt_blobs": blobs, "component_maps": components, "cases": cases}


def verify(out, config, codec, repeat_of=None):
    raw = json.loads((out / "observations.raw.json").read_text(encoding="utf-8"))
    require(raw["gameplay_ticks"] == 0 and raw["menu_creation_exercised"] is False and raw["player"] is None, "unexpected runtime authority")
    require(raw["world_seed_i64"] == config["world_seed_i64"], "wrong native world seed")
    specs = {case["id"]: case for case in config["cases"]}
    require([case["id"] for case in raw["cases"]] == list(specs), "missing/reordered lifecycle cases")
    checks = [verify_case(case, specs[case["id"]], codec) for case in raw["cases"]]
    data = compact(raw, config, codec)
    write_json(out / "chest-lifecycle-26.1.json", data)
    fixture_path = out / "chest-lifecycle-26.1.json"
    require(json.loads(fixture_path.read_text(encoding="utf-8")) == data, "fixture JSON readback differs")
    repeated = False
    if repeat_of:
        require(fixture_path.read_bytes() == (repeat_of / fixture_path.name).read_bytes(), "independent native lifecycle fixture differs")
        require(raw == json.loads((repeat_of / "observations.raw.json").read_text(encoding="utf-8")), "independent raw lifecycle observations differ")
        repeated = True
    result = {"status": "pass", "cases": checks, "case_count": len(checks), "native_fills": sum(case["first_fill_count"] for case in checks),
              "native_loads": 2 * len(checks), "independent_repeat_equal": repeated,
              "fixture_sha256": digest(fixture_path.read_bytes()), "nbt_blob_count": len(data["nbt_blobs"]),
              "item_component_map_count": len(data["component_maps"]),
              "no_rng_scope": "Main-thread native load/save/null-unpack/getItem windows; instrumented RNG creation/draw methods, fill calls, context creation, and all captured loot RNG states."}
    write_json(out / "verification.json", result)
    return result
