"""Summarize frozen desert-pyramid replays without weakening oracle comparisons."""
import argparse
import hashlib
import json
from pathlib import Path


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def replay(directory, native_sha):
    provenance = load(directory / "provenance.json")
    comparison = load(directory / "comparison.json")
    for path, key in ((directory / "native-history-replay.exe", "binary_sha256"),
                      (directory / "source.zip", "source_zip_sha256"),
                      (directory / "chunks.jsonl", "result_sha256"),
                      (directory / "replay.json", "replay_sha256")):
        if sha(path) != provenance[key]:
            raise ValueError(f"frozen artifact changed: {path}")
    if (comparison["rust_output_sha256"] != provenance["result_sha256"]
            or provenance["native_provenance_sha256"] != native_sha
            or comparison["native_provenance_sha256"] != native_sha):
        raise ValueError("native/replay/comparison provenance mismatch")
    rows = []
    for row in comparison["requests"]:
        light = row["serialized_light"]
        metadata = row["generated_metadata"]
        required = [row["states"], row["biomes"], row["source_order"],
                    *(metadata[k] for k in ("structures", "block_entity_payloads", "postprocessing"))]
        if any("mismatches" not in field for field in required):
            raise ValueError("a required comparison was not scored")
        rows.append({"request": row["request"], "spec": row["spec"],
            "states": row["states"]["mismatches"], "state_total": row["states"]["total"],
            "first_state_difference": row["states"]["first"],
            "biomes": row["biomes"]["mismatches"], "biome_total": row["biomes"]["total"],
            "source_order": row["source_order"]["mismatches"],
            "structures": metadata["structures"]["mismatches"],
            "block_entities": metadata["block_entity_payloads"]["mismatches"],
            "block_entity_examples": metadata["block_entity_payloads"]["examples"],
            "ordered_marks": metadata["postprocessing"]["mismatches"],
            "light_scored": light["scored"], "light_matches": light.get("matches"),
            "light_presence": light.get("presence_mismatches", 0),
            "light_lengths": light.get("length_mismatches", 0),
            "light_bytes": light.get("byte_mismatches", 0),
            "wg_presence": sum(not value["presence_matches"] for value in row["worldgen_heightmaps"].values()),
            "wg_values": sum(value.get("values", {}).get("mismatches", 0) for value in row["worldgen_heightmaps"].values()),
            "bcore_complete": row["bcore_complete"], "stages": row["bcore_stages"]})
    totals = {key: sum(row[key] for row in rows) for key in (
        "states", "state_total", "biomes", "biome_total", "source_order", "structures",
        "block_entities", "ordered_marks", "light_presence", "light_lengths", "light_bytes", "wg_presence", "wg_values")}
    totals["scored_light_snapshots"] = sum(row["light_scored"] for row in rows)
    totals["matching_light_snapshots"] = sum(row["light_matches"] is True for row in rows)
    totals["complete_requests"] = sum(row["bcore_complete"] for row in rows)
    return {"directory": str(directory), "binary_sha256": provenance["binary_sha256"],
        "source_zip_sha256": provenance["source_zip_sha256"], "result_sha256": provenance["result_sha256"],
        "comparison_sha256": sha(directory / "comparison.json"), "totals": totals, "requests": rows,
        "limitations": comparison["limitations"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--before", type=Path)
    parser.add_argument("--after", type=Path)
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--repeat", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    native_sha = sha(args.native / "provenance.json")
    capture = load(args.native / "capture.json")
    if native_sha != capture["provenance_sha256"] or sha(args.native / "events.jsonl") != capture["events_sha256"]:
        raise ValueError("native provenance changed")
    result = {"native": str(args.native), "native_provenance_sha256": native_sha,
              "native_capture": capture, "audit_sha256": sha(Path(__file__))}
    for side in ("before", "after"):
        if directory := getattr(args, side):
            result[side] = replay(directory, native_sha)
    if args.before and args.after:
        if [r["spec"] for r in result["before"]["requests"]] != [r["spec"] for r in result["after"]["requests"]]:
            raise ValueError("before/after histories differ")
        result["state_improvement"] = result["before"]["totals"]["states"] - result["after"]["totals"]["states"]
    if args.fixture:
        data = load(args.fixture)
        repeat = load(args.repeat)
        a, b = dict(data), dict(repeat)
        for item in (a, b):
            item.pop("capture_script_sha256", None)
        if a != b:
            raise ValueError("native component repetitions differ")
        passes = [p for row in data["placements"] for p in row["passes"]]
        result["component"] = {
            "fixture_sha256": sha(args.fixture), "repeat_sha256": sha(args.repeat),
            "probe_sha256": data["probe_sha256"],
            "canonical_sha256": hashlib.sha256(json.dumps(a, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
            "admissions": len(data["admission"]), "histories": len(data["placements"]), "passes": len(passes),
            "writes": sum(p["write_count"] for p in passes),
            "references": sum(len(row["targets"]) for row in data["references"]),
            "loot_observations": sum(e[0] == "loot" for p in passes for e in p["effects"]),
            "ordered_marks": sum(e[0] == "mark" for p in passes for e in p["effects"]),
            "fluid_ticks": sum(len(p["ticks"]) for p in passes),
            "block_entity_snapshots": sum(len(p["block_entities"]) for p in passes),
        }
    with args.output.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] if key not in ("before", "after") else result[key]["totals"]
                     for key in result if key != "native_capture"}, indent=2))


if __name__ == "__main__":
    main()
