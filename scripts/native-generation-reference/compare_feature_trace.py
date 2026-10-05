"""Compare production feature-boundary inputs and results with a native case."""
import argparse
import json
from pathlib import Path

from oracle import mismatch, nested_mismatch, sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("case", type=Path)
    parser.add_argument("replay", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    native = json.loads(args.case.read_text(encoding="utf-8"))
    trace_file = args.replay / "feature_traces.jsonl"
    traces = [json.loads(line) for line in trace_file.read_text(encoding="utf-8").splitlines()]
    if len(traces) != 2 or [row["boundary"] for row in traces] != ["before", "after"]:
        raise ValueError("expected exactly one complete production feature trace")
    result = {"native_case_sha256": sha(args.case), "bcore_trace_sha256": sha(trace_file), "boundaries": []}
    for actual in traces:
        if actual["source"] != native["source"] or actual["feature"] != native["feature"]:
            raise ValueError("unmatched production/native source feature")
        boundary = actual["boundary"]
        rng = native["entry" if boundary == "before" else "exit"]["rng"]["next_i64_from_copy"]
        row = {"boundary": boundary, "rng_matches": actual["next_i64"] == rng,
               "native_next_i64": rng, "bcore_next_i64": actual["next_i64"], "chunks": []}
        chunks = {tuple(chunk["pos"]): chunk for chunk in actual["chunks"]}
        if chunks.keys() != {tuple(chunk["pos"]) for chunk in native[boundary]}:
            raise ValueError("production/native feature dependency chunk set differs")
        for expected in native[boundary]:
            x, z = expected["pos"]
            observed = chunks[x, z]
            row["chunks"].append({"pos": [x,z],
                "states": mismatch(expected["states"], observed["states"],
                    lambda i: [x * 16 + i % 16, -64 + i // 256, z * 16 + i // 16 % 16]),
                "biomes": mismatch(expected["biomes"], observed["biomes"],
                    lambda i: [x * 4 + i % 4, -16 + i // 16, z * 4 + i // 4 % 4]),
                "worldgen_heightmaps": nested_mismatch(
                    {key: value for key, value in expected["heightmaps"].items() if key.endswith("_WG")},
                    observed["worldgen_heightmaps"])})
        result["boundaries"].append(row)
    with args.output.open("x", encoding="utf-8") as out:
        json.dump(result, out, indent=2)
    print(json.dumps({"output": str(args.output), "boundaries": [
        {"boundary": row["boundary"], "rng_matches": row["rng_matches"], "chunks": [
            {"pos": chunk["pos"], "states": chunk["states"]["mismatches"],
             "biomes": chunk["biomes"]["mismatches"], "wg": chunk["worldgen_heightmaps"]["mismatches"],
             "first_state": chunk["states"]["first"], "first_biome": chunk["biomes"]["first"]}
            for chunk in row["chunks"]]} for row in result["boundaries"]]}, indent=2))


if __name__ == "__main__":
    main()
