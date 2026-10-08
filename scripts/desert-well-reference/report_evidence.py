"""Collect preserved history results/hashes without changing any evidence."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--replay", action="append", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    assert not a.output.exists()
    result = {"replays": [], "components": {}}
    for directory in a.replay:
        provenance = json.loads((directory / "provenance.json").read_text())
        comparison = json.loads((directory / "comparison.json").read_text())
        chunks = [row for line in (directory / "chunks.jsonl").read_text().splitlines()
                  if (row := json.loads(line))["phase"] == "requests"]
        rows = []
        for row, chunk in zip(comparison["requests"], chunks):
            rows.append({"request": row["request"], "spec": row["spec"], "states": row["states"]["mismatches"],
                         "biomes": row["biomes"]["mismatches"], "source_order": row["source_order"]["mismatches"],
                         "worldgen_heightmaps": row["worldgen_heightmaps"], "serialized_light": row["serialized_light"],
                         "metadata": row["generated_metadata"], "native_effect_counts": row["native_effect_counts"],
                         "bcore_complete": row["bcore_complete"], "stages": row["bcore_stages"],
                         "feature_omissions": [{"source": s["source"], "missing": s["missing"]} for s in chunk["feature_sources"]],
                         "bcore_effect_counts": {k: len(chunk["snapshot"].get(k) or []) for k in
                             ("block_entities", "tick_requests", "entities", "structure_entities", "postprocessing")}})
        result["replays"].append({"directory": str(directory), "hashes": {n: sha(directory / n) for n in
            ("source.zip", "provenance.json", "native-history-replay.exe", "chunks.jsonl", "comparison.json", "replay.json")},
            "provenance": {k: v for k, v in provenance.items() if k != "source_sha256"}, "requests": rows})
    for name in ("components-02", "components-repeat-01"):
        path = ROOT / "target/desert-well" / name / "result.json"
        data = json.loads(path.read_text())
        result["components"][name] = {"sha256": sha(path), "samples": len(data["samples"]),
            "names": [s["name"] for s in data["samples"]],
            "writes": sum(sum(e["op"] == "write" for e in s["events"]) for s in data["samples"]),
            "reads": sum(sum(e["op"] == "read" for e in s["events"]) for s in data["samples"]),
            "lookups": sum(sum(e["op"] == "lookup" for e in s["events"]) for s in data["samples"])}
    a.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(a.output)
    for run in result["replays"]:
        print(run["directory"], [{"states": r["states"], "biomes": r["biomes"], "complete": r["bcore_complete"]} for r in run["requests"]])


if __name__ == "__main__":
    main()
