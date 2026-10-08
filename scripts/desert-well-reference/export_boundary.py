"""Export native well boundaries including (rather than dropping) its effects."""
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/native-generation-reference"))
from feature_case import heightmaps
from oracle import decode_snapshot, load_events, sha, state_key


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("capture", type=Path)
    p.add_argument("--source", default="-223,-250")
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    assert not a.output.exists()
    source = list(map(int, a.source.split(",")))
    events = load_events(a.capture)
    snapshots = {e["boundary"]: e for e in events if e["event"] == "feature_snapshot"
                 and e["feature"] == "minecraft:desert_well" and e["source"] == source}
    assert set(snapshots) == {"before", "after"}
    by_id = {e["seq"]: e for e in events}
    registry = {state_key(s): s["id"] for s in json.loads((a.capture / "block-states.json").read_text())}
    result = {"schema": 1, "source": source, "feature": "minecraft:desert_well",
              "seed": json.loads((a.capture / "config.json").read_text())["seed"],
              "native_provenance": sha(a.capture / "provenance.json"),
              "native_events": sha(a.capture / "events.jsonl"), "exporter_sha256": sha(Path(__file__))}
    for boundary, event in snapshots.items():
        result["entry" if boundary == "before" else "exit"] = by_id[event["feature_event"]]
        chunks = []
        for snapshot in event["chunks"]:
            data = decode_snapshot(a.capture, snapshot, registry)
            chunks.append({"pos": snapshot["pos"], "states": data["states"], "biomes": data["biomes"],
                           "heightmaps": heightmaps(data["extra"]["heightmaps_live"], data["min_y"]),
                           "nbt": data["nbt"], "extra": data["extra"], "native_snapshot": snapshot})
        result[boundary] = chunks
    result["effects"] = [e for e in events if result["entry"]["seq"] < e["seq"] < result["exit"]["seq"]
                         and (e["event"].startswith("world_") or e["event"].startswith("chunk_"))]
    a.output.write_text(json.dumps(result, separators=(",", ":")) + "\n", encoding="utf-8", newline="\n")
    print("Preserved native 3x3 state/biome/NBT/heightmap boundaries and", len(result["effects"]), "effects:", a.output)


if __name__ == "__main__":
    main()
