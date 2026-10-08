"""Read-only native history verification and desert-well event extraction."""
import argparse
from collections import Counter
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/native-generation-reference"))
from oracle import load_events, sha, summarize


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("capture", type=Path)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    assert not a.output.exists()
    events = load_events(a.capture)
    wells = []
    for entry in events:
        if entry["event"] != "feature_enter" or entry["feature"] != "minecraft:desert_well":
            continue
        exit_event = next(e for e in events if e["seq"] > entry["seq"] and e["event"] == "feature_exit"
                          and e["feature"] == entry["feature"] and e["source"] == entry["source"])
        actions = [e for e in events if entry["seq"] < e["seq"] < exit_event["seq"]]
        writes = [e for e in actions if e["event"] == "world_set_block"]
        wells.append({"entry": entry, "exit": exit_event, "writes": writes,
                      "accepted": sum(e["accepted"] for e in writes),
                      "effects": dict(Counter(e["event"] for e in actions))})
    result = {"summary": summarize(a.capture, verify=True), "wells": wells,
              "capture_sha256": sha(a.capture / "capture.json"),
              "events_sha256": sha(a.capture / "events.jsonl"),
              "provenance_sha256": sha(a.capture / "provenance.json"),
              "analyzer_sha256": sha(Path(__file__))}
    a.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"output": str(a.output), "wells": [
        {"source": w["entry"]["source"], "placed": w["exit"]["result"], "accepted": w["accepted"],
         "first_write": w["writes"][0] if w["writes"] else None} for w in wells]}, indent=2))


if __name__ == "__main__":
    main()
