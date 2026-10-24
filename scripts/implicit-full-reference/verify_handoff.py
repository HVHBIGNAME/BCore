"""Check the bounded native handoff assertions against retained capture evidence.

This checks the particular fixtures, not a production scheduler implementation.
Run through scripts/run_generation_check.py --background.
"""

import argparse
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from analyze import ROOT, digest, save


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--analysis", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output, analysis = args.output.resolve(), args.analysis.resolve()
    if not output.is_relative_to(ROOT / "target") or output.exists():
        parser.error("output must be a new target directory")
    here = Path(__file__).resolve().parent
    scenario = load(here / "scenario.json")
    checkpoint = load(ROOT / scenario["request_source"].split(":")[0])
    historical = next(row for row in checkpoint["histories"] if row["name"] == scenario["historical_name"])
    requests = [{"pos": row["chunk"], "status": row["status"]} for row in historical["requests"]]
    require(requests == scenario["requests"], "reconstructed requests differ from checkpoint")
    comparison = load(analysis / "repeat-comparison.json")
    require(comparison["native_identity_equal"], "repeat native dependencies differ")
    require(comparison["feature_source_order_equal"], "repeat source order differs")
    require(len(comparison["snapshots"]) == 21, "seven requests and two watches expected")
    for row in comparison["snapshots"]:
        require(row["raw_nbt_equal"] and row["raw_extra_equal"], "repeat raw snapshot differs")
        require(all(row["fields_equal"].values()), "repeat decoded snapshot differs")
    reports = []
    for name in ("implicit-full-native-20261015-01", "implicit-full-native-20261015-02"):
        summary_path = analysis / (name + ".summary.json")
        summary = load(summary_path)
        capture = Path(summary["capture"])
        require(digest(capture / "events.jsonl") == summary["receipt"]["events_sha256"], "trace changed")
        require(summary["config"]["requests"] == requests, "native request sequence differs")
        for key in ("seed", "bootstrap", "processors", "workers"):
            require(summary["config"][key] == scenario["new_capture_settings"][key], f"setting differs: {key}")
        require(summary["terminal"]["game_time"] == summary["terminal"]["gameplay_ticks"] == 0,
                "gameplay time entered")
        pending = summary["terminal"]["inflight_stages"]
        require(len(pending) == 1 and pending[0]["pos"] == [1, 0] and pending[0]["status"] == "minecraft:full",
                "unexpected terminal stage prefix")
        returned = [row["requested"] for row in summary["requests"]]
        require([row["nbt_status"] for row in returned] == ["minecraft:" + status for status in
                ("features", "initialize_light", "spawn", "initialize_light", "initialize_light", "spawn", "full")],
                "observed request snapshot status differs")
        transition, = summary["full_serialization_transitions"]
        require(transition["block_entity_payload_changes"]["mismatches"] == 36, "BE payload transition differs")
        require(transition["removed_live_heightmaps"] == ["OCEAN_FLOOR_WG", "WORLD_SURFACE_WG"], "WG transition differs")
        for key in ("states_equal", "biomes_equal", "saved_ticks_equal", "postprocessing_equal"):
            require(transition[key], f"FULL transition changed {key}")
        require(returned[6]["saved_tick_counts"]["block_ticks"] == 616, "saved block tick count differs")
        require(returned[6]["postprocessing_count"] == 1847, "postprocessing mark count differs")
        require(summary["requests"][6]["future_api_calls"] == 1, "final LIGHT did not take the future path")
        order_path = analysis / (name + ".partial-order.json")
        order = load(order_path)
        require(not order["unmatched_dispatcher_handoffs"], "unmatched native dispatcher handoff")
        for family in ("same_thread_edges_in_selected_spine", "matched_dispatcher_submission_to_handoff", "stage_entry_to_completion"):
            require(all(edge["before"] < edge["after"] for edge in order[family]), f"reversed evidence edge: {family}")
        reports.append({"capture": str(capture), "summary_sha256": digest(summary_path),
                        "partial_order_sha256": digest(order_path),
                        "matched_dispatcher_handoffs": len(order["matched_dispatcher_submission_to_handoff"]),
                        "inflight_stages": pending})
    baseline = load(analysis / "full-boundaries-native-20261014-01.summary.json")
    cached = baseline["requests"][3]
    require(cached["future_api_calls"] == 0, "explicit-FULL contrast lacks cache-only return")
    require(cached["requested"]["chunk_class"].endswith(".ProtoChunk"), "cached object is not the old proto")
    require(cached["watch"][0]["chunk_class"].endswith(".LevelChunk"), "watched holder is not converted")
    require(cached["requested"]["reference"]["chunk"] != cached["watch"][0]["reference"]["chunk"],
            "cached and watched identities unexpectedly coincide")
    output.mkdir(parents=True)
    for name in ("verify_handoff.py", "analyze.py", "scenario.json"):
        (output / name).write_bytes((here / name).read_bytes())
    record = {"schema": 1, "status": "pass", "analysis": str(analysis), "captures": reports,
              "repeat_comparison_sha256": digest(analysis / "repeat-comparison.json"),
              "sources": {name: digest(output / name) for name in ("verify_handoff.py", "analyze.py", "scenario.json")},
              "scope": "Assertions about these native fixtures only; no BCore execution or global parity claim."}
    save(output / "verification.json", record)
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    main()
