"""Package an immutable native capture; never recompute its expected results."""

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    source = args.capture.resolve()
    destination = args.destination.resolve()
    if not source.is_relative_to(ROOT / "target") or not destination.is_relative_to(ROOT):
        parser.error("all inputs and outputs must remain in the workspace")
    document = source / "generation-spawn.json"
    data = json.loads(document.read_text(encoding="utf-8"))
    assert data["minecraft"] == "26.1" and data["protocol"] == 775
    assert data["gameplay_ticks"] == 0
    for case in data["cases"]:
        assert all(entity["problems"] == "" for entity in case["entities"])
    data["provenance"] = json.loads((source / "provenance.json").read_text(encoding="utf-8"))
    data["provenance"]["capture_sha256"] = hashlib.sha256(document.read_bytes()).hexdigest()
    data["provenance"]["capture_directory"] = str(source.relative_to(ROOT)).replace("\\", "/")
    with destination.open("x", encoding="utf-8") as file:
        json.dump(data, file, separators=(",", ":"))
        file.write("\n")
    print(f"Packed {len(data['cases'])} native cases: {destination}")


if __name__ == "__main__":
    main()
