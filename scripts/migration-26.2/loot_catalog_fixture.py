"""Replay historical inputs and package newly observed 26.2 native fill results."""
from __future__ import annotations

from contextlib import contextmanager
import importlib.util
from pathlib import Path
import struct
import sys

import native
import loot_catalog_data as data

HISTORICAL_FIXTURE = native.ROOT / "crates/bcore-worldgen/tests/data/loot_26_1/loot-fill-26.1.json"
HISTORICAL_SHA256 = "2158fd855f1bbc46d6b399620c9ced35822263c982151686f58e175c1ccf9db1"
VERIFIER_SOURCES = {
    "scripts/loot-reference/inventory.py": "620541f1c6e9d340db9b232ff62400969de2794f70f3a5e612a6a0b8d832d709",
    "scripts/loot-reference/verify.py": "03402a57958ad0971a4099a9ae5c34ef85061f539d972ba344e663ab9a7734b4",
    "scripts/loot-reference/audit.py": "7795ba3e5032111880376e98e13e85fd3d17f612f474a721d38cdf48db37b0e4",
    "scripts/loot-reference/package.py": "c7003cf4577db14922c366cbf57e4e5d10e4d8921c23737c6abe0c6960a23e29",
}


def signed_i64(value, label: str, *, nonzero: bool = False) -> int:
    data.require(isinstance(value, str), f"{label}: signed-i64 values must be decimal strings")
    integer = int(value)
    data.require(str(integer) == value and -(1 << 63) <= integer < 1 << 63
                 and (integer != 0 or not nonzero), f"{label}: invalid signed-i64 value")
    return integer


def validate_input(spec: dict) -> None:
    data.require(isinstance(spec, dict) and type(spec.get("schema")) is int and spec["schema"] == 1,
                 "fill inputs require schema 1")
    signed_i64(spec["world_seed"], "world seed")
    context = spec["context"]
    luck = context["luck"]
    data.require(type(luck) in (int, float) and luck == 0 and struct.pack(">f", luck) == b"\0\0\0\0"
                 and context["player"] is None, "fill inputs require positive-zero luck and no player")
    origin = context["origin_block"]
    data.require(isinstance(origin, list) and len(origin) == 3
                 and all(type(value) is int and -(1 << 31) <= value < 1 << 31 for value in origin),
                 "origin must contain three signed-i32 block coordinates")
    cases = spec["cases"]
    data.require(isinstance(cases, list) and 1 <= len(cases) <= 256, "fill inputs require 1..256 explicit cases")
    ids = set()
    for case in cases:
        name = case["id"]
        data.require(isinstance(name, str) and name and name not in ids, "duplicate or empty case ID")
        ids.add(name)
        data.resource(case["table"], name)
        signed_i64(case["seed"], name + " seed", nonzero=True)
        size = case["size"]
        data.require(type(size) is int and 0 <= size <= 54, name + ": container size outside 0..54")
        data.require(case.get("missing_table_policy") in (None, "native_lookup"), name + ": unknown missing-table policy")
        slots = case["preexisting"]
        data.require(isinstance(slots, list), name + ": preexisting slots must be a list")
        indices = [slot["slot"] for slot in slots]
        data.require(len(indices) == len(set(indices)) and all(type(slot) is int and 0 <= slot < size for slot in indices),
                     name + ": duplicate or out-of-range preexisting slot")
        data.require(all("stack" in slot for slot in slots), name + ": missing preexisting stack")


def historical_inputs(fixture: dict) -> dict:
    data.require(fixture["schema"] == "bcore.native-loot-fill.v1" and fixture["minecraft"] == "26.1"
                 and fixture["protocol"] == 775 and len(fixture["cases"]) == 137, "wrong historical input fixture")
    cases = fixture["cases"]
    context = cases[0]["context"]
    data.require(all(case["context"] == context for case in cases), "historical input contexts differ")
    data.require(context["dimension"] == "minecraft:overworld" and context["luck_f32_bits"] == "0"
                 and context["player"] is None and context["gameplay_ticks"] == 0, "unsupported historical input context")
    inputs = []
    for case in cases:
        spec = {"id": case["id"], "table": case["table"], "seed": case["seed_i64"],
                "size": case["container_size"], "preexisting": [
                    {"slot": slot, "stack": stack} for slot, stack in enumerate(case["preexisting_slots"]) if stack is not None]}
        data.require(len(case["preexisting_slots"]) == spec["size"], "historical backing array/size disagree")
        if case["table_resolution"] == "native_reloadable_empty_fallback":
            spec["missing_table_policy"] = "native_lookup"
        else:
            data.require(case["table_resolution"] == "registered", "unknown historical table-resolution input")
        inputs.append(spec)
    result = {"schema": 1, "world_seed": context["world_seed_i64"],
              "context": {"origin_block": context["block_pos"], "luck": 0, "player": None}, "cases": inputs}
    validate_input(result)
    return result


def input_spec(mode: str, cases: Path | None) -> tuple[dict, dict]:
    if mode == "catalog":
        data.require(cases is None, "--cases applies only to fill")
        return {"schema": 1, "world_seed": "846692123413862008", "context": {
            "origin_block": [8, 80, 8], "luck": 0, "player": None}, "cases": []}, {"authority": "catalog-only"}
    source = HISTORICAL_FIXTURE if cases is None else cases.resolve()
    raw = source.read_bytes()
    data.require(len(raw) <= data.JSON_LIMIT, "case input exceeds its JSON bound")
    spec = data.decode(raw)
    authority = {"source": str(source), "source_sha256": data.digest(raw), "authority": "explicit-inputs"}
    if cases is None:
        data.require(data.digest(raw) == HISTORICAL_SHA256, "historical fixture SHA-256 changed")
        spec = historical_inputs(spec)
        authority["authority"] = "historical-26.1-input-replay-on-26.2"
    validate_input(spec)
    data.require(set(spec) == {"schema", "world_seed", "context", "cases"}, "unexpected fill input root fields")
    return spec, authority


@contextmanager
def frozen_verifiers(directory: Path):
    saved = {name: sys.modules.get(name) for name in ("inventory", "verify", "audit", "package")}
    loaded = {}
    try:
        for name in saved:
            path = directory / f"sources/scripts/loot-reference/{name}.py"
            spec = importlib.util.spec_from_file_location(name, path)
            if spec is None or spec.loader is None:
                raise ValueError(f"cannot load frozen loot verifier: {path}")
            module = importlib.util.module_from_spec(spec)
            sys.modules[name] = module
            spec.loader.exec_module(module)
            loaded[name] = module
        yield loaded
    finally:
        for name, module in saved.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module


def verify_fill(directory: Path, spec: dict, repeat: Path | None) -> dict:
    observations = data.load(directory / "observations.raw.json")
    data.identity(observations, "native fill observations")
    data.require(observations["world_seed"] == spec["world_seed"] and observations["context"] == spec["context"]
                 and [case["input"] for case in observations["cases"]] == spec["cases"], "observed fill inputs differ")
    with frozen_verifiers(directory) as helpers:
        verification = helpers["verify"].verify_capture(directory, repeat)
        data.require(not verification["missing_cases"] and verification["case_count"] == len(spec["cases"]),
                     "requested native fill cases were not captured")
        data.require(verification["native_fills"] == len(spec["cases"]) + sum(case["size"] == 27 for case in spec["cases"]),
                     "native direct/unpack route count differs from the explicit inputs")
        context = {"world_seed_i64": observations["world_seed"], "dimension": "minecraft:overworld",
                   "block_pos": spec["context"]["origin_block"],
                   "origin": [value + 0.5 for value in spec["context"]["origin_block"]],
                   "luck_f32_bits": "0", "player": None, "gameplay_ticks": observations["gameplay_ticks"]}
        records = [helpers["package"].fixture(case, directory, context) for case in observations["cases"]]
        for record in records:
            helpers["package"].check_nbt(record)
    portable = {"schema": "bcore.native-loot-fill.v1", "minecraft": "26.2", "protocol": 776,
                "jar_sha256": native.JAR_SHA256,
                "authority": {"capture": str(directory), "capture_inputs_sha256": native.sha(directory / "capture-inputs.json"),
                              "independent_passes": 2 if repeat is not None else 1,
                              "fixtures_sha256": verification["fixtures_sha256"]},
                "cases": records}
    native.write_json(directory / "loot-fill-26.2.json", portable)
    return verification
