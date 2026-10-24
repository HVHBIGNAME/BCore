"""Read-only acceptance check for the portable fixture delivery and its authorities."""

import argparse
import ast
import json
from pathlib import Path
import zipfile

from audit import load, verify_manifest
from inventory import ROOT, digest, new_output, write_json
from package import check_nbt
from verify import MASK, check_continuation, require


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = new_output(args.output)
    verified_files = verify_manifest(args.package)
    data = load(args.package / "loot-fill-26.1.json")
    require(data["schema"] == "bcore.native-loot-fill.v1", "unexpected portable schema")
    tables = load(args.package / "tables.json")
    items = {item["key"]: item for item in load(args.package / "items.json")["items_native_order"]}
    sources = {}
    for authority in data["authority"]:
        path = ROOT / "target" / authority["first"]
        require(digest((path / "fixtures.json").read_bytes()) == authority["fixtures_sha256"], "native fixture identity differs")
        sources[path.name] = {case["input"]["id"]: case for case in load(path / "fixtures.json")["cases"]}
    slots = components = 0
    for case in data["cases"]:
        seed = int(case["seed_i64"])
        require(-(1 << 63) <= seed < (1 << 63) and seed != 0, "uncovered seed in portable fixtures")
        require(isinstance(case["container_size"], int), "portable slot count is not an integer")
        require(case["context"]["player"] is None and case["context"]["luck_f32_bits"] == "0", "unexpected context")
        require(len(case["expected_slots"]) == len(case["preexisting_slots"]) == case["container_size"], "portable slot-array shape differs")
        native = sources[case["authority"]["capture"]][case["authority"]["case_id"]]
        require(case["expected_slot_details"] == native["direct"]["after"], "portable reduction differs from the native fixture")
        require(case["placement_order"] == native["direct"]["placements"], "portable placement order differs")
        if case["table_resolution"] == "registered":
            require(case["table"] in tables, "registered fixture has no frozen table input")
        else:
            require(case["table"] not in tables and not case["generated_stacks"], "invalid native missing-table fallback")
        for item in case["expected_slots"]:
            if item is None:
                continue
            require(item["id"] in items and isinstance(item["count"], int) and item["count"] > 0, "invalid portable native item")
            slots += 1
            components += len(item.get("components", {}))
        for name in ("rng_after_generation", "rng_after_slot_shuffle", "rng_after_split_shuffle", "rng_after_fill"):
            require(0 <= int(case[name]["seed48"]) <= MASK, "invalid captured LCG state")
        check_continuation({"rng_after": case["rng_after_fill"], "continuation": case["rng_continuation"]})
        check_nbt(case)
    with zipfile.ZipFile(args.package / "evidence.zip") as archive:
        require(archive.testzip() is None, "evidence archive CRC mismatch")
        require(archive.read("integration/loot-fill-26.1.json") == (args.package / "loot-fill-26.1.json").read_bytes(), "archive fixture bytes differ")
    source_hashes = {}
    for path in sorted(Path(__file__).parent.iterdir()):
        if path.is_file():
            raw = path.read_bytes()
            source_hashes[path.name] = digest(raw)
            if path.suffix == ".py":
                ast.parse(raw, filename=str(path))
    jobs = []
    for path in sorted((ROOT / "target/generation-checks").glob("loot-native-20261016-*.json")):
        if "final-check" in path.name:
            continue
        status = load(path)
        require(status["state"] == "finished" and status.get("exit_code") == 0, f"loot research job incomplete: {path.name}")
        jobs.append({**status, "status_path": str(path), "status_sha256": digest(path.read_bytes())})
    report = {"status": "pass", "package": str(args.package.resolve()), "verified_manifest_files": verified_files,
              "portable_cases": len(data["cases"]), "occupied_expected_slots": slots, "component_patch_entries": components,
              "source_hashes": source_hashes, "completed_jobs": jobs,
              "fixture_path": str((args.package / "loot-fill-26.1.json").resolve()),
              "fixture_sha256": digest((args.package / "loot-fill-26.1.json").read_bytes())}
    write_json(out / "acceptance.json", report)
    with (out / "check_package.py").open("xb") as stream:
        stream.write(Path(__file__).read_bytes())
    print(json.dumps({"status": "pass", "cases": len(data["cases"]), "jobs": len(jobs), "output": str(out),
                      "occupied_slots": slots, "component_patch_entries": components}, indent=2))


if __name__ == "__main__":
    main()
