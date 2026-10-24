"""Offline validation of versioned native loot captures and promotion inputs."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path, PurePosixPath
import re

import native

ENGINE_FILES = ("tables.json", "items.json", "enchantment.json", "instrument.json", "potions.json", "registry-types.json")
RUNTIME_FILES = (*ENGINE_FILES, "loaded-tables.json", "table-inventory.json", "loot-catalog.json")
HISTORICAL_JAVA = {
    "scripts/loot-reference/LootAgent.java": "b0d6a6b47a3027baf730e9ba2ca412eb033db7227c3fc29cf64d1f2b26010ff2",
    "scripts/loot-reference/LootProbe.java": "42a64b642d34b3d00be34383010147795958f721d7fe0921fc41644e4c427985",
    "scripts/native-generation-reference/NativeAccess.java": "93f32d9a715ea0b82abfe262c5362efc0ad08c25cfa3c028d3c5bc792c5fd55f",
}
JSON_LIMIT = 32 * 1024 * 1024
TRACE_LIMIT = 256 * 1024 * 1024
CAPTURE_LIMIT = 512 * 1024 * 1024
CAPTURE_SCHEMA = "bcore.migration.loot-capture.v1"
CATALOG_SCHEMA = "bcore.native-loot-catalog.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def digest(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def object_pairs(pairs) -> dict:
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON member: {key}")
        result[key] = value
    return result


def nonfinite(value: str):
    raise ValueError(f"nonfinite JSON number: {value}")


def decode(raw: bytes):
    return json.loads(raw, object_pairs_hook=object_pairs, parse_constant=nonfinite)


def load(path: Path, limit: int = JSON_LIMIT):
    require(path.is_file() and path.stat().st_size <= limit, f"missing or oversized JSON: {path}")
    return decode(path.read_bytes())


def identity(value: dict, label: str) -> None:
    require(isinstance(value, dict), f"{label}: expected an object")
    for key, expected in (("minecraft", "26.2"), ("protocol", 776), ("jar_sha256", native.JAR_SHA256)):
        require(value.get(key) == expected, f"{label}: different {key}; refusing to relabel historical data")


def resource(key: str, label: str) -> str:
    require(isinstance(key, str) and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9/_.-]+", key) is not None,
            f"{label}: invalid resource identifier {key!r}")
    return key


def unique_keys(rows: list, label: str) -> list[str]:
    require(isinstance(rows, list), f"{label}: expected native-order rows")
    keys = [resource(row["key"], label) for row in rows]
    require(len(keys) == len(set(keys)), f"{label}: duplicate registry key")
    return keys


def registry_ids(rows: list, label: str) -> list[str]:
    keys = unique_keys(rows, label)
    require(all(type(row["id"]) is int and row["id"] == index for index, row in enumerate(rows)),
            f"{label}: native numeric IDs are not contiguous in native order")
    return keys


def validate_tags(rows: list, keys: list[str], label: str) -> None:
    require(isinstance(rows, list), f"{label}: expected native tag rows")
    seen, members = set(), set(keys)
    for row in rows:
        key = resource(row["tag"], label)
        require(key not in seen, f"{label}: duplicate tag {key}")
        seen.add(key)
        values = row["members_native_order"]
        require(isinstance(values, list) and len(values) == len(set(values)) and set(values) <= members,
                f"{label}: duplicate or unknown native holder in {key}")


def validate_catalog(data: dict, registry: dict) -> dict:
    catalog = data["loot-catalog.json"]
    identity(catalog, "native loot catalog")
    identity(registry, "independent native registry")
    require(catalog["schema"] == CATALOG_SCHEMA and catalog["mode"] in ("catalog", "fill"), "wrong loot catalog schema/mode")
    require(catalog["gameplay_ticks"] == 0 and catalog["data_version"] == registry["data_version"] == 4903,
            "loot catalog must be captured at zero gameplay ticks on data version 4903")
    require(catalog["lookup_source"] == "MinecraftServer.reloadableRegistries().lookup()"
            and catalog["level_class"] == "net.minecraft.server.level.ServerLevel"
            and catalog["server_class"] == "net.minecraft.server.dedicated.DedicatedServer",
            "loot exports require the initialized native server's reloadable lookup")
    version = catalog["native_version"]
    require((version["name"], version["protocol_version"], version["world_version"]) == ("26.2", 776, 4903),
            "native loaded version identity differs")

    item_data = data["items.json"]
    items = item_data["items_native_order"]
    item_keys = registry_ids(items, "items")
    validate_tags(item_data["tags"], item_keys, "item tags")
    types = data["registry-types.json"]
    expected_types = {"DATA_COMPONENT_TYPE", "LOOT_POOL_ENTRY_TYPE", "LOOT_FUNCTION_TYPE", "LOOT_CONDITION_TYPE",
                      "LOOT_NUMBER_PROVIDER_TYPE", "LOOT_NBT_PROVIDER_TYPE", "LOOT_SCORE_PROVIDER_TYPE"}
    require(set(types) == expected_types, "incomplete native loot/component registry type inventory")
    for name, rows in types.items():
        registry_ids(rows, name)
    components = {row["key"]: row["persistent"] for row in types["DATA_COMPONENT_TYPE"]}
    require(all(type(persistent) is bool for persistent in components.values()), "component persistence flags are not booleans")
    for row in items:
        defaults = row["components"]["persistent"]
        transient = row["components"]["unserialized_transient_types"]
        require(isinstance(defaults, dict) and set(defaults) <= components.keys()
                and all(components[key] for key in defaults), f"{row['key']}: uncatalogued persistent defaults")
        require(isinstance(transient, list) and len(transient) == len(set(transient))
                and all(key in components and not components[key] for key in transient),
                f"{row['key']}: invalid transient component inventory")
        require(type(row["max_stack_size"]) is int and row["max_stack_size"] > 0,
                f"{row['key']}: invalid native maximum stack size")
        if row["key"] != "minecraft:air":
            require(defaults["minecraft:max_stack_size"] == row["max_stack_size"],
                    f"{row['key']}: default component and native stack maximum disagree")

    counts = {"items": len(items), "item_tags": len(item_data["tags"]), "registry_types": {k: len(v) for k, v in types.items()}}
    for name, key in (("enchantment.json", "enchantments"), ("instrument.json", "instruments")):
        values = data[name]
        keys = unique_keys(values["values_native_order"], name)
        require(all(isinstance(row["value"], dict) for row in values["values_native_order"]), f"{name}: missing native codec values")
        validate_tags(values["tags"], keys, name + " tags")
        counts[key], counts[key + "_tags"] = len(keys), len(values["tags"])
    counts["potions"] = len(registry_ids(data["potions.json"], "potions"))
    for name, rows in (("ITEM", items), ("DATA_COMPONENT_TYPE", types["DATA_COMPONENT_TYPE"]), ("POTION", data["potions.json"])):
        expected = registry["registries"][name]
        require([(row["id"], row["key"]) for row in rows] == [(row["id"], row["key"]) for row in expected],
                f"{name}: loot and independent 26.2 runtime registry IDs disagree")

    tables, loaded = data["tables.json"], data["loaded-tables.json"]
    require(isinstance(tables, dict) and all(isinstance(value, dict) for value in tables.values()), "invalid native table codecs")
    for key in tables:
        resource(key, "loot tables")
    require(loaded["all_loaded_keys"] == sorted(tables), "incomplete loaded/native-codec loot table inventory")
    candidates = sorted(key for key in tables if key.startswith("minecraft:chests/") or "trial_chambers/" in key)
    rows = loaded["chest_and_trial_tables"]
    require([row["key"] for row in rows] == candidates, "incomplete chest/trial native table inventory")
    require(all(row["native_codec"] == tables[row["key"]] for row in rows), "chest/trial and all-table native codec exports disagree")
    inventory = data["table-inventory.json"]
    identity(inventory, "official JAR table inventory")
    require(inventory["all_jar_keys"] == sorted(tables), "official JAR and loaded vanilla table membership disagree")
    require(inventory["chest_and_trial_keys"] == candidates, "official JAR chest/trial membership differs")
    counts.update({"loaded_tables": len(tables), "chest_and_trial_tables": len(candidates)})
    require(counts == catalog["counts"], "native catalog count checks disagree with the complete exports")
    return counts


def manifest_path(directory: Path, name: str) -> Path:
    require(isinstance(name, str), "capture manifest paths must be strings")
    part = PurePosixPath(name)
    require(str(part) == name and name and not part.is_absolute()
            and ".." not in part.parts and "\\" not in name and ":" not in name, f"unsafe capture manifest path: {name}")
    path = (directory / name).resolve()
    require(path.is_relative_to(directory.resolve()), f"capture manifest escapes its directory: {name}")
    return path


def verified_capture(directory: Path, expected_manifest: str) -> tuple[dict, dict]:
    require(not (directory / "failure.json").exists(), "loot capture retained a failure record")
    path = directory / "manifest.json"
    require(native.sha(path) == expected_manifest, "capture manifest differs from the supplied SHA-256 pin")
    manifest = load(path)
    identity(manifest, "loot capture manifest")
    require(manifest["schema"] == CAPTURE_SCHEMA and manifest["status"] == "pass", "loot capture is incomplete or failed")
    files = manifest["files_sha256"]
    require(isinstance(files, dict) and len(files) <= 1024, "loot capture manifest exceeds its file bound")
    total = 0
    for name, expected in files.items():
        file = manifest_path(directory, name)
        require(file.is_file(), f"missing captured file: {name}")
        size = file.stat().st_size
        limit = TRACE_LIMIT if name == "trace.jsonl" or name.endswith(".log") else JSON_LIMIT
        require(size <= limit, f"captured file exceeds its bound: {name}")
        total += size
        require(total <= CAPTURE_LIMIT, "loot capture exceeds its total byte bound")
        require(native.sha(file) == expected, f"captured input changed: {name}")
    required = {*RUNTIME_FILES, "capture-inputs.json", "input-spec.json", "config.json", "environment.json", "native-result.json",
                "instrumentation.json", "sources/registry_26_2.json", "sources/registry-provenance_26_2.json", "trace.jsonl"}
    require(required <= files.keys(), "incomplete loot capture manifest membership")
    for name, expected in HISTORICAL_JAVA.items():
        require(files.get("sources/" + name) == expected, f"historical Java source changed: {name}")
    data = {name: load(directory / name) for name in RUNTIME_FILES}
    registry = load(directory / "sources/registry_26_2.json")
    registry_provenance = load(directory / "sources/registry-provenance_26_2.json")
    identity(registry_provenance, "independent registry provenance")
    require(registry_provenance["generated_sha256"]["registries_26_2.json"] == files["sources/registry_26_2.json"],
            "captured registry differs from its independent publication digest")
    counts = validate_catalog(data, registry)
    require(manifest["counts"] == counts and manifest["mode"] == data["loot-catalog.json"]["mode"], "capture manifest/counts disagree")
    require(load(directory / "native-result.json") == data["loot-catalog.json"], "native marker/catalog result differs")
    inputs = load(directory / "capture-inputs.json")
    identity(inputs, "loot capture inputs")
    require(inputs["mode"] == manifest["mode"] and inputs["historical_source_sha256"] == HISTORICAL_JAVA,
            "loot capture input authority differs")
    for name, key in (("input-spec.json", "input_spec_sha256"), ("environment.json", "environment_sha256"),
                      ("sources/registry_26_2.json", "registry_sha256"),
                      ("sources/registry-provenance_26_2.json", "registry_provenance_sha256"),
                      ("tools/asm-9.8.jar", "asm_sha256")):
        require(files.get(name) == inputs[key], f"capture input manifest digest differs: {name}")
    for name, expected in inputs["source_sha256"].items():
        require(files.get("sources/" + name) == expected, f"frozen source manifest digest differs: {name}")
    instrumentation = load(directory / "instrumentation.json")
    methods = instrumentation["methods"]
    require(any(method.startswith("server/MinecraftServer.setInitialSpawn(") for method in methods)
            and any(method.startswith("server/MinecraftServer.tickServer(") for method in methods)
            and any(method.startswith("server/level/ServerLevel.tick(") for method in methods),
            "native bootstrap or gameplay-tick guard was not instrumented")
    environment = load(directory / "environment.json")
    require(environment["jar_sha256"] == native.JAR_SHA256 and len(environment["libraries"]) == 39,
            "native capture environment lacks the authenticated 26.2 server/libraries")
    spec = load(directory / "input-spec.json")
    require(data["loot-catalog.json"]["world_seed_i64"] == spec["world_seed"], "observed world seed differs from the input")
    config = load(directory / "config.json")
    identity(config, "native server capture config")
    require(config["mode"] == manifest["mode"] and all(config.get(key) == value for key, value in spec.items()),
            "native server config differs from its frozen input spec")
    if manifest["mode"] == "fill":
        from loot_catalog_fixture import VERIFIER_SOURCES

        for name, expected in VERIFIER_SOURCES.items():
            require(files.get("sources/" + name) == expected, f"historical loot verifier changed: {name}")
        require({"loot-fill-26.2.json", "verification.json", "observations.raw.json", "fixtures.json"} <= files.keys(),
                "fill capture lacks checked native observations")
        fixture = load(directory / "loot-fill-26.2.json")
        identity(fixture, "native fill fixture")
        verification = load(directory / "verification.json")
        require(fixture["schema"] == "bcore.native-loot-fill.v1" and verification["status"] == "pass"
                and not verification["missing_cases"] and len(fixture["cases"]) == verification["case_count"]
                == data["loot-catalog.json"]["case_count"] == manifest["case_count"]
                and verification["native_fills"] == manifest["native_fills"], "fill capture has missing or unverified cases")
        require(verification["fixtures_sha256"] == files["fixtures.json"]
                and fixture["authority"]["fixtures_sha256"] == files["fixtures.json"]
                and fixture["authority"]["capture_inputs_sha256"] == files["capture-inputs.json"],
                "portable fixture authority differs from its checked native observations")
        require([case["id"] for case in fixture["cases"]] == [case["id"] for case in spec["cases"]],
                "portable fixture dropped or reordered explicit case inputs")
    else:
        require(data["loot-catalog.json"]["case_count"] == data["loot-catalog.json"]["event_count"] == manifest["case_count"]
                == manifest["native_fills"] == 0 and not spec["cases"]
                and (directory / "trace.jsonl").stat().st_size == 0, "catalog capture includes fill events")
    return manifest, data
