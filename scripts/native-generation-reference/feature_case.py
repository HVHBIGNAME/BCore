"""Export one observed placed-feature boundary, with every input/output block."""

import argparse
import json
from pathlib import Path

from oracle import decode_snapshot, load_events, sha, state_key


def heightmaps(raw, min_y):
    return {name: [min_y + ((words[i // 7] >> ((i % 7) * 9)) & 511) for i in range(256)]
            for name, words in raw.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--source", default="0,0")
    parser.add_argument("--feature", default="minecraft:patch_grass_forest")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source = list(map(int, args.source.split(",")))
    events = load_events(args.capture)
    registry = {state_key(s): s["id"] for s in json.loads((args.capture / "block-states.json").read_text())}
    snapshots = {e["boundary"]: e for e in events if e["event"] == "feature_snapshot"
                 and e["source"] == source and e["feature"] == args.feature}
    if set(snapshots) != {"before", "after"}:
        raise ValueError("requires native --feature-snapshots for this source and feature")
    by_id = {e["seq"]: e for e in events}
    entry = by_id[snapshots["before"]["feature_event"]]
    exit_event = by_id[snapshots["after"]["feature_event"]]
    result = {"schema": 1, "seed": json.loads((args.capture / "config.json").read_text())["seed"],
              "source": source, "feature": args.feature, "entry": entry, "exit": exit_event,
              "native_provenance": sha(args.capture / "provenance.json"), "native_events": sha(args.capture / "events.jsonl"),
              "exporter_sha256": sha(Path(__file__)), "decoder_sha256": sha(Path(__file__).with_name("oracle.py")),
              "write_observation": "WorldGenRegion.setBlock only; direct LevelChunkSection writes require the full pre/post snapshots"}
    for boundary, event in snapshots.items():
        result[boundary] = []
        for snapshot in event["chunks"]:
            data = decode_snapshot(args.capture, snapshot, registry)
            result[boundary].append({"pos": snapshot["pos"], "states": data["states"], "biomes": data["biomes"],
                                     "min_y": data["min_y"], "heightmaps": heightmaps(data["extra"]["heightmaps_live"], data["min_y"]),
                                     "is_light_on": data["nbt"].get("isLightOn", False), "native_snapshot": snapshot})
    result["writes"] = [[*e["pos"], e["state"], e["flags"]] for e in events if entry["seq"] < e["seq"] < exit_event["seq"]
                        and e["event"] == "world_set_block" and e["accepted"]]
    result["other_effects"] = [e for e in events if entry["seq"] < e["seq"] < exit_event["seq"]
                               and e["event"] in ("world_tick_request", "world_postprocess", "chunk_postprocess", "world_entity")]
    if result["other_effects"]:
        raise ValueError("this forest-grass diagnostic does not support unexpected additional effects")
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, separators=(",", ":"))
    print(f"Exported {args.feature} at {source}: {len(result['writes'])} writes, 9 complete pre/post chunks: {args.output}")


if __name__ == "__main__":
    main()
