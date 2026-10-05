"""Inspect captured native spawn evidence and assert fixture structural invariants."""

import argparse
from collections import Counter
import json
from pathlib import Path


def plain(tag):
    kind, payload = tag
    if kind == 10:
        return {key: plain(value) for key, value in payload.items()}
    if kind == 9:
        return [plain(value) for value in payload]
    return payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path)
    parser.add_argument("--case")
    parser.add_argument("--entity", type=int, default=0)
    parser.add_argument("--full", action="store_true")
    args = parser.parse_args()
    data = json.loads(args.fixture.read_text())
    counts = Counter(entity["type"] for case in data["cases"] for entity in case["entities"])
    if not args.case:
        print(json.dumps({"cases": len(data["cases"]), "biomes": len(data["catalog"]["biomes"]),
                          "entity_counts": counts, "types": data["catalog"]["types"],
                          "sound_variants": data["catalog"]["sound_variants"]}, indent=2))
    for case in data["cases"]:
        if args.case and case["input"]["name"] != args.case:
            continue
        assert all(entity["problems"] == "" for entity in case["entities"])
        if args.case:
            print(json.dumps({"input": case["input"], "spawn_count": len(case["entities"]),
                "entities": [{"nbt": plain(entity["typed_nbt"]), "rotation": entity["rotation"],
                    "head_yaw": entity["head_yaw"]} for i, entity in enumerate(case["entities"])
                    if args.full or i == args.entity],
                "rng": case["rng"] if args.full else [rng for rng in case["rng"] if rng["stream"] == f"entity:{args.entity}"],
                "group_results": [event for event in case["events"] if "finalize_exit" in event] if args.full else
                    Counter(str(event["group"]) for event in case["events"] if "finalize_exit" in event)}, indent=2))
        elif case["entities"] or case["input"].get("forced"):
            print(case["input"]["name"], "entities=", len(case["entities"]), "top_queries=",
                  sum("top" in event for event in case["events"]), "draws=",
                  {rng["stream"]: len(rng["draws"]) for rng in case["rng"]})


if __name__ == "__main__":
    main()
