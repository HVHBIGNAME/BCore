"""Summarize native request/object boundaries and compare independent histories.

Run through scripts/run_generation_check.py --background after oracle verify.
The original event streams and typed blobs remain the authority. Observation
sequence is retained as evidence coordinates, not treated as thread causality.
"""

import argparse
from collections import Counter, defaultdict, deque
import hashlib
import importlib.util
import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[2]
STATUSES = {"minecraft:" + name for name in ("features", "initialize_light", "light", "spawn", "full")}
sys.dont_write_bytecode = True


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical_digest(value):
    data = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(data).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def freeze_oracle(output):
    source = ROOT / "scripts/native-generation-reference/oracle.py"
    frozen = output / "oracle.py"
    frozen.write_bytes(source.read_bytes())
    spec = importlib.util.spec_from_file_location("implicit_full_frozen_oracle", frozen)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def snapshot_summary(oracle, directory, snapshot, registry):
    if snapshot.get("absent"):
        return snapshot
    decoded = oracle.decode_snapshot(directory, snapshot, registry)
    nbt, extra = decoded["nbt"], decoded["extra"]
    entities = nbt.get("block_entities", [])
    ticks = {name: nbt.get(name, []) for name in ("block_ticks", "fluid_ticks")}
    return {
        "reference": snapshot,
        "chunk_class": extra["chunk_class"],
        "nbt_status": nbt["Status"],
        "live_heightmaps": sorted(extra["heightmaps_live"]),
        "saved_heightmaps": sorted(nbt.get("Heightmaps", {})),
        "block_entity_count": len(entities),
        "block_entities": entities,
        "saved_tick_counts": {name: len(rows) for name, rows in ticks.items()},
        "saved_ticks_sha256": canonical_digest(ticks),
        "postprocessing_count": sum(map(len, nbt.get("PostProcessing", []))),
        "postprocessing_sha256": canonical_digest(nbt.get("PostProcessing", [])),
        "states_sha256": canonical_digest(decoded["states"]),
        "biomes_sha256": canonical_digest(decoded["biomes"]),
    }


def lifecycle_transition(oracle, stage):
    before = stage["before"]["snapshot"]
    after = stage["after"]["snapshot"]

    def payloads(snapshot):
        result = {}
        for entity in snapshot["block_entities"]:
            key = ",".join(str(entity[axis]) for axis in ("x", "y", "z"))
            if key in result:
                raise ValueError("duplicate block entity position")
            result[key] = {name: value for name, value in entity.items() if name != "keepPacked"}
        return result

    return {
        "stage_id": stage["entry"]["seq"], "pos": stage["entry"]["pos"],
        "before_seq": stage["before"]["seq"], "after_seq": stage["after"]["seq"],
        "removed_live_heightmaps": sorted(set(before["live_heightmaps"]) - set(after["live_heightmaps"])),
        "block_entity_payload_changes": oracle.nested_mismatch(payloads(before), payloads(after)),
        "states_equal": before["states_sha256"] == after["states_sha256"],
        "biomes_equal": before["biomes_sha256"] == after["biomes_sha256"],
        "saved_ticks_equal": before["saved_ticks_sha256"] == after["saved_ticks_sha256"],
        "postprocessing_equal": before["postprocessing_sha256"] == after["postprocessing_sha256"],
        "boundary": "FULL stage observer before/after serialization; conversion and save are not separately measured here",
    }


def inspect(oracle, directory, output):
    config = json.loads((directory / "config.json").read_text())
    capture = json.loads((directory / "capture.json").read_text())
    if digest(directory / "events.jsonl") != capture["events_sha256"]:
        raise ValueError("events digest differs from capture receipt")
    if digest(directory / "provenance.json") != capture["provenance_sha256"]:
        raise ValueError("provenance digest differs from capture receipt")
    provenance = json.loads((directory / "provenance.json").read_text())
    for source, expected in provenance["sources"].items():
        if digest(directory / "build" / Path(source).name) != expected:
            raise ValueError(f"frozen observer source digest mismatch: {source}")
    if digest(directory / "build/capture.py") != provenance["runner_sha256"]:
        raise ValueError("frozen capture runner digest mismatch")
    registry = {oracle.state_key(row): row["id"] for row in
                json.loads((directory / "block-states.json").read_text())}
    watch = {tuple(pos) for pos in config["watch"]}
    requests, stages, sources, counts = {}, {}, [], Counter()
    terminal = None
    snapshot_cache = {}
    dispatch_pending, dispatch_edges = defaultdict(deque), []
    unmatched_dispatches = []
    thread_tail, thread_edges = {}, []

    def summarize_snapshot(snapshot):
        if snapshot.get("absent"):
            return snapshot
        key = snapshot["nbt"], snapshot["extra"]
        if key not in snapshot_cache:
            snapshot_cache[key] = snapshot_summary(oracle, directory, snapshot, registry)
        return {**snapshot_cache[key], "reference": snapshot}

    filtered_path = output / (directory.name + ".scheduler.jsonl")
    index = 0
    with (directory / "events.jsonl").open(encoding="utf-8") as stream, filtered_path.open("w", encoding="utf-8") as filtered:
        for index, line in enumerate(stream, 1):
            event = json.loads(line)
            if event["seq"] != index:
                raise ValueError("non-contiguous event sequence")
            kind = event["event"]
            counts[kind] += 1
            request = event["request"]
            watched = tuple(event.get("pos", [])) in watch
            relevant_stage = watched and event.get("status") in STATUSES
            if kind == "dispatcher_submit":
                dispatch_pending[event["dispatcher"], event["runnable"]].append(event["seq"])
            elif kind == "dispatcher_execute":
                for runnable in event["tasks"]:
                    pending = dispatch_pending[event["dispatcher"], runnable]
                    if pending:
                        dispatch_edges.append({"before": pending.popleft(), "after": event["seq"],
                                               "dispatcher": event["dispatcher"], "runnable": runnable})
                    else:
                        unmatched_dispatches.append({"seq": event["seq"], "runnable": runnable})
            if kind == "request_enter":
                requests[request] = {"enter": event, "native_calls": []}
            elif kind == "native_request" and request in requests:
                requests[request]["native_calls"].append(event)
            elif kind == "request_exit":
                requests[request].update({
                    "exit_seq": event["seq"],
                    "requested": summarize_snapshot(event["snapshot"]),
                    "watch": [summarize_snapshot(snapshot) for snapshot in event["watch"]],
                })
            elif kind == "stage_enter":
                if event["status"] == "minecraft:features":
                    sources.append({key: event[key] for key in ("seq", "request", "pos", "source_sequence", "thread")})
                if relevant_stage:
                    stages[event["seq"]] = {"entry": event}
            elif kind == "stage_exit" and event["stage_id"] in stages:
                stages[event["stage_id"]]["exit"] = event
            elif kind == "stage_snapshot" and event["stage_id"] in stages:
                center = next(row["snapshot"] for row in event["chunks"]
                              if row["snapshot"]["pos"] == event["pos"])
                stages[event["stage_id"]][event["boundary"]] = {
                    "seq": event["seq"], "snapshot": summarize_snapshot(center)}
            elif kind == "oracle_complete":
                terminal = event
            retained = (kind.startswith("request_") or kind == "native_request"
                    or relevant_stage and kind != "stage_snapshot"
                    or watched and event.get("target") == "minecraft:full"
                    or watched and kind.startswith("dispatcher_"))
            if retained:
                filtered.write(line)
                thread = event["thread"]
                if thread in thread_tail:
                    thread_edges.append({"before": thread_tail[thread], "after": event["seq"], "thread": thread})
                thread_tail[thread] = event["seq"]
    if terminal is None or terminal["seq"] != index:
        raise ValueError("capture lacks a terminal oracle_complete")
    for row in requests.values():
        if "exit_seq" not in row:
            raise ValueError("request lacks a return snapshot")
        calls = row["native_calls"]
        row["future_api_calls"] = sum(call["api"].endswith(".getChunkFutureMainThread") for call in calls)
        row["get_chunk_calls"] = sum(call["api"].endswith(".getChunk") for call in calls)
    summary = {
        "capture": str(directory), "config": config, "receipt": capture,
        "native_identity": {key: provenance[key] for key in
                            ("jar_sha256", "sources", "runner_sha256", "asm_sha256", "java", "libraries")},
        "event_counts": dict(counts), "requests": list(requests.values()),
        "watched_stages": list(stages.values()), "feature_sources": sources,
        "terminal": terminal, "scheduler_excerpt": filtered_path.name,
        "scheduler_excerpt_sha256": digest(filtered_path),
        "full_serialization_transitions": [lifecycle_transition(oracle, stage) for stage in stages.values()
                                           if stage["entry"]["status"] == "minecraft:full" and "after" in stage],
        "limits": [
            "Request and phase fields are observation-time labels, not the originating cause of asynchronous work.",
            "Watch snapshots precede the requested snapshot; a request_exit row is not an atomic cross-thread snapshot.",
            "The base observer lacks explicit main-executor enqueue/run and cache clear/store events.",
            "In-flight terminal stages remain pending; this analysis does not drain native work.",
        ],
    }
    save(output / (directory.name + ".partial-order.json"), {
        "events_sha256": capture["events_sha256"],
        "same_thread_edges_in_selected_spine": thread_edges,
        "matched_dispatcher_submission_to_handoff": dispatch_edges,
        "unmatched_dispatcher_handoffs": unmatched_dispatches,
        "submissions_not_observed_handed_off": [seq for pending in dispatch_pending.values() for seq in pending],
        "stage_entry_to_completion": [{"before": stage["entry"]["seq"], "after": stage["exit"]["seq"]}
                                      for stage in stages.values() if "exit" in stage],
        "limits": ["dispatcher_execute observes scheduleForExecution entry, not runnable-body completion.",
                   "Edges cite same-thread order, explicit dispatcher/runnable identity, or stage ID only.",
                   "No cross-thread edge is inferred solely from adjacent sequence values or request labels."],
    })
    save(output / (directory.name + ".summary.json"), summary)
    return summary


def compare(left, right):
    keys = ("seed", "bootstrap", "requests", "watch", "processors", "workers", "feature_snapshots")
    if any(left["config"][key] != right["config"][key] for key in keys):
        raise ValueError("independent repeat uses a different scenario")
    rows = []
    for before, after in zip(left["requests"], right["requests"], strict=True):
        pairs = [("requested", before["requested"], after["requested"])]
        pairs.extend((f"watch[{index}]", a, b) for index, (a, b) in
                     enumerate(zip(before["watch"], after["watch"], strict=True)))
        for surface, a, b in pairs:
            if a.get("absent") or b.get("absent"):
                rows.append({"request": before["enter"]["request"], "surface": surface,
                             "absence_equal": a == b})
                continue
            fields = ("chunk_class", "nbt_status", "live_heightmaps", "saved_heightmaps",
                      "block_entities", "saved_ticks_sha256", "postprocessing_sha256",
                      "states_sha256", "biomes_sha256")
            rows.append({
                "request": before["enter"]["request"], "surface": surface,
                "pos": a["reference"]["pos"],
                "raw_nbt_equal": a["reference"]["nbt"] == b["reference"]["nbt"],
                "raw_extra_equal": a["reference"]["extra"] == b["reference"]["extra"],
                "fields_equal": {field: a[field] == b[field] for field in fields},
            })
    right_stages = {(tuple(stage["entry"]["pos"]), stage["entry"]["status"]): stage
                    for stage in right["watched_stages"]}
    timing = []
    for stage in left["watched_stages"]:
        counterpart = right_stages.get((tuple(stage["entry"]["pos"]), stage["entry"]["status"]))
        if counterpart:
            timing.append({"pos": stage["entry"]["pos"], "status": stage["entry"]["status"],
                           "entry_request": [stage["entry"]["request"], counterpart["entry"]["request"]],
                           "completion_request": [stage.get("exit", {}).get("request"),
                                                  counterpart.get("exit", {}).get("request")]})
    return {"left": left["capture"], "right": right["capture"], "snapshots": rows,
            "native_identity_equal": left["native_identity"] == right["native_identity"],
            "stage_observation_labels": timing,
            "feature_source_order_equal": [row["pos"] for row in left["feature_sources"]]
            == [row["pos"] for row in right["feature_sources"]]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compare-first-two", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / "target") or output.exists():
        parser.error("output must be a new target directory")
    captures = [path.resolve() for path in args.capture]
    if len({path.name for path in captures}) != len(captures):
        parser.error("capture directory names must be distinct")
    if args.compare_first_two and len(captures) < 2:
        parser.error("repeat comparison requires at least two captures")
    output.mkdir(parents=True)
    (output / "analyze.py").write_bytes(Path(__file__).read_bytes())
    oracle = freeze_oracle(output)
    summaries = [inspect(oracle, path, output) for path in captures]
    if args.compare_first_two:
        save(output / "repeat-comparison.json", compare(*summaries[:2]))
    save(output / "provenance.json", {
        "runner_sha256": digest(output / "analyze.py"),
        "frozen_oracle_sha256": digest(output / "oracle.py"),
        "captures": {str(path): {name: digest(path / name) for name in
                     ("capture.json", "config.json", "provenance.json", "events.jsonl")}
                     for path in captures},
    })
    print(json.dumps({"output": str(output), "captures": len(captures)}))


if __name__ == "__main__":
    main()
