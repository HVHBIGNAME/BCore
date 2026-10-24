"""Compare historical template-BE input identities with captured target state flags."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

HISTORICAL_JAR = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
TARGET_JAR = "183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8"


def identity(spec: dict) -> tuple:
    name = spec.get("Name")
    properties = spec.get("Properties", {})
    if not isinstance(name, str) or not isinstance(properties, dict) or any(
            not isinstance(key, str) or not isinstance(value, str)
            for key, value in properties.items()):
        raise ValueError("block state must have a resource name and string properties")
    return name, tuple(sorted(properties.items()))


def indexed_states(catalog: dict) -> tuple[list, dict]:
    rows = catalog["states"]
    by_identity = {}
    for index, row in enumerate(rows):
        if type(row[0]) is not int or row[0] != index or type(row[4]) is not int:
            raise ValueError(f"invalid ordered block state or flags at {index}")
        key = identity(row[1])
        if key in by_identity:
            raise ValueError(f"duplicate complete block state identity: {key}")
        by_identity[key] = row
    return rows, by_identity


def audit(fixture: dict, old: dict, target: dict) -> dict:
    for catalog, version, jar in ((fixture, "26.1", HISTORICAL_JAR),
                                  (old, "26.1", HISTORICAL_JAR),
                                  (target, "26.2", TARGET_JAR)):
        if (catalog.get("minecraft"), catalog.get("jar_sha256")) != (version, jar):
            raise ValueError(f"captured {version} identity differs")
    historical, _ = indexed_states(old)
    _, target_states = indexed_states(target)
    differences = []
    rows = fixture["template_block_entities"]
    for index, row in enumerate(rows):
        state = row["state"]
        if type(state) is not int or not 0 <= state < len(historical):
            raise ValueError(f"historical template state is unavailable at row {index}")
        previous = historical[state]
        key = identity(previous[1])
        if key not in target_states:
            raise ValueError(f"target block state identity is unavailable: {key}")
        current = target_states[key]
        has_default = current[1]["Name"] in target["block_entities"]
        if current[4] & 8 == 0 or not has_default:
            differences.append({"row": index, "template": row["template"],
                                "old_state": previous[0], "target_state": current[0],
                                "identity": current[1], "flags": current[4],
                                "has_default": has_default})
    return {"rows": len(rows), "differences": differences}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path)
    parser.add_argument("historical_registry", type=Path)
    parser.add_argument("target_registry", type=Path)
    args = parser.parse_args()
    inputs = [path.read_bytes() for path in
              (args.fixture, args.historical_registry, args.target_registry)]
    fixture, old, target = map(json.loads, inputs)
    if len(old["states"]) != 29873 or len(target["states"]) != 32366:
        parser.error("audit requires complete historical and target block-state inventories")
    print(json.dumps({"inputs_sha256": [hashlib.sha256(raw).hexdigest() for raw in inputs],
                      **audit(fixture, old, target)}, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
