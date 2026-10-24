"""Verify native blobs, companion provenance/links, and actual snapshot differences.

Invoke through the main heavy queue. All verification outputs are new artifacts;
the input captures and shared observer/decoder sources are read-only.
"""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import zipfile

sys.dont_write_bytecode = True
from admission_contract import analyze_sidecar, object_id, require
from capture_admission import ASM_SHA256, BASELINE_SOURCES, JAR_SHA256, ROOT, load, save, sha


def baseline_events(directory):
    result = {"stages": {}, "stage_exits": {}, "requests": [], "sources": []}
    with (directory / "events.jsonl").open(encoding="utf-8") as stream:
        for line in stream:
            event = json.loads(line)
            kind = event["event"]
            if kind == "stage_enter":
                result["stages"][event["seq"]] = event
                if event["status"] == "minecraft:features":
                    result["sources"].append(event["pos"])
            elif kind == "stage_exit":
                result["stage_exits"][event["stage_id"]] = event
            elif kind == "request_exit":
                result["requests"].append(event)
            elif kind == "oracle_complete":
                result["terminal"] = event
    require("terminal" in result, "baseline terminal event is absent")
    return result


def provenance_checks(directory):
    provenance = load(directory / "provenance.json")
    receipt = load(directory / "capture.json")
    sidecar = load(directory / "admission.json")
    require(sha(directory / "provenance.json") == receipt["provenance_sha256"], "provenance digest mismatch")
    require(sha(directory / "capture.json") == sidecar["baseline_capture_sha256"], "baseline capture receipt changed")
    require(sidecar["native_exit_code"] == 0, "native process failed")
    require(sha(directory / "admission-events.jsonl") == sidecar["events_sha256"], "sidecar digest mismatch")
    require(provenance["jar_sha256"] == JAR_SHA256, "native JAR identity differs")
    require(provenance["asm_sha256"] == ASM_SHA256 == sha(directory / "build/asm-9.8.jar"), "ASM identity differs")
    source_manifest = {Path(path).name: value for path, value in provenance["sources"].items()}
    require(source_manifest == BASELINE_SOURCES, "baseline source contract changed")
    for group in ("sources", "companion_sources"):
        for path, expected in provenance[group].items():
            require(sha(directory / "build" / Path(path).name) == expected, f"frozen source changed: {path}")
    require(sha(directory / "build/capture.py") == provenance["runner_sha256"], "frozen runner changed")
    for path, expected in provenance["class_sha256"].items():
        require(sha(directory / "build" / path) == expected, f"compiled class changed: {path}")
    for name, key, classes in (("history-agent.jar", "agent_sha256", "baseline-classes"),
                               ("admission-agent.jar", "companion_agent_sha256", "companion-classes")):
        jar = directory / "build" / name
        require(sha(jar) == provenance[key], f"agent JAR changed: {name}")
        with zipfile.ZipFile(jar) as archive:
            for path, expected in provenance["class_sha256"].items():
                if Path(path).parts[0] == classes:
                    raw = archive.read(Path(path).name)
                    require(hashlib.sha256(raw).hexdigest() == expected, "JAR class differs from compiled manifest")
    agents = [arg for arg in provenance["command"] if arg.startswith("-javaagent:")]
    require(len(agents) == 2 and "history-agent.jar=" in agents[0] and "admission-agent.jar" in agents[1],
            "agent order differs from baseline-then-companion contract")
    require(sha(directory / "config.json") == provenance["config_sha256"], "capture config changed")
    return provenance, sidecar


def freeze_decoder(output):
    frozen = output / "baseline_oracle.py"
    frozen.write_bytes((ROOT / "scripts/native-generation-reference/oracle.py").read_bytes())
    spec = importlib.util.spec_from_file_location("private_admission_frozen_oracle", frozen)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def indexed_entities(nbt, packing=False):
    rows = {}
    for entity in nbt.get("block_entities", []):
        key = ",".join(str(entity[axis]) for axis in ("x", "y", "z"))
        require(key not in rows, "duplicate BE position in saved snapshot")
        rows[key] = {name: value for name, value in entity.items() if (name == "keepPacked") == packing}
    return rows


def compare_snapshots(oracle, reference, capture, reference_events, capture_events):
    reference_config, config = load(reference / "config.json"), load(capture / "config.json")
    for key in ("seed", "bootstrap", "requests", "watch", "processors", "workers", "feature_snapshots"):
        require(reference_config[key] == config[key], f"scenario differs for snapshot comparison: {key}")
    registry = {oracle.state_key(row): row["id"] for row in load(capture / "block-states.json")}
    require(load(reference / "block-states.json") == load(capture / "block-states.json"), "native state registry differs")
    cached = {}

    def decode(directory, snapshot):
        key = str(directory), snapshot["nbt"], snapshot["extra"]
        if key not in cached:
            cached[key] = oracle.decode_snapshot(directory, snapshot, registry)
        return cached[key]

    comparisons = []
    for left, right in zip(reference_events["requests"], capture_events["requests"], strict=True):
        require(left["spec"] == right["spec"], "request snapshot specs differ")
        pairs = [("requested", left["snapshot"], right["snapshot"])]
        pairs.extend((f"watch[{i}]", a, b) for i, (a, b) in enumerate(zip(left["watch"], right["watch"], strict=True)))
        for surface, a, b in pairs:
            item = {"request": right["request"], "surface": surface, "pos": a["pos"],
                    "reference_seq": left["seq"], "capture_seq": right["seq"]}
            if a.get("absent") or b.get("absent"):
                item.update(reference_absent=bool(a.get("absent")), capture_absent=bool(b.get("absent")),
                            equal=bool(a.get("absent")) == bool(b.get("absent")))
                comparisons.append(item)
                continue
            x, y = decode(reference, a), decode(capture, b)
            require(len(x["states"]) == len(y["states"]) and len(x["biomes"]) == len(y["biomes"]), "snapshot dimensions differ")
            item.update(
                raw_nbt_equal=a["nbt"] == b["nbt"], raw_extra_equal=a["extra"] == b["extra"],
                reference_status=x["nbt"]["Status"], capture_status=y["nbt"]["Status"],
                reference_class=x["extra"]["chunk_class"], capture_class=y["extra"]["chunk_class"],
                state_differences=sum(a != b for a, b in zip(x["states"], y["states"], strict=True)),
                biome_differences=sum(a != b for a, b in zip(x["biomes"], y["biomes"], strict=True)),
                reference_live_heightmaps=sorted(x["extra"]["heightmaps_live"]),
                capture_live_heightmaps=sorted(y["extra"]["heightmaps_live"]),
                nbt_differences=oracle.nested_mismatch(x["nbt"], y["nbt"]),
                extra_differences=oracle.nested_mismatch(x["extra"], y["extra"]),
                be_payload_differences=oracle.nested_mismatch(indexed_entities(x["nbt"]), indexed_entities(y["nbt"])),
                be_packing_differences=oracle.nested_mismatch(indexed_entities(x["nbt"], True), indexed_entities(y["nbt"], True)),
            )
            item["equal"] = item["raw_nbt_equal"] and item["raw_extra_equal"]
            comparisons.append(item)
    return {"reference": str(reference), "capture": str(capture), "normalization": "none",
        "source_order_equal": reference_events["sources"] == capture_events["sources"],
        "snapshots": comparisons, "different_snapshot_pairs": sum(not row["equal"] for row in comparisons),
        "state_differences": sum(row.get("state_differences", 0) for row in comparisons),
        "biome_differences": sum(row.get("biome_differences", 0) for row in comparisons)}


def compact_report(directory, native, analysis, comparison):
    return {"capture": str(directory), "baseline_verified_blobs": native["verified_blobs"],
        "sidecar_events": sum(analysis["event_counts"].values()),
        "main_runnable_links": len(analysis["main_runnable_links"]),
        "executions_before_schedule_return": sum(link["execution_before_schedule_return"] for link in analysis["main_runnable_links"]),
        "scheduled_not_observed_executed": len(analysis["scheduled_not_observed_executed"]),
        "baseline_inflight_stages": native["inflight_stages"],
        "primary_apis": [{key: row[key] for key in ("request_label", "api_call_id", "enter_seq", "return_seq", "pos", "requested_status", "returned", "path", "counters")}
                         for row in analysis["primary_apis"]],
        "pumps": [{key: row[key] for key in ("request_label", "enter_seq", "return_seq", "future_id", "ready_at_entry", "counters", "pending_ids_observed_at_both_boundaries")}
                  for row in analysis["pumps"]],
        "accessibility": analysis["accessibility"],
        "full_chains": [{"pos": row["pos"], "baseline_stage_id": row["baseline_stage_id"],
            "full_call_id": row["full_call_id"], "supplier_id": object_id(row["supplier"]),
            "native_future_id": object_id(row["native_full_future"]),
            "runnable_id": object_id(row["schedule"]["enter"]["runnable"]) if row["schedule"] else None,
            "schedule_enter_seq": row["schedule"]["enter"]["seq"] if row["schedule"] else None,
            "schedule_return_seq": row["schedule"].get("return", {}).get("seq") if row["schedule"] else None,
            "execution": row["execution"],
            "body_enter_seq": row["body"]["enter"]["seq"] if row["body"] else None,
            "body_return_seq": row["body"].get("return", {}).get("seq") if row["body"] else None,
            "returned": row["body"].get("return", {}).get("returned") if row["body"] else None,
            "full_status_supplier_id": object_id(row["status_supplier"]["enter"]["supplier"]) if row["status_supplier"] else None,
            "baseline_exit_seq": row["baseline_stage_exit"]["seq"] if row["baseline_stage_exit"] else None}
            for row in analysis["full_chains"]],
        "snapshot_comparison": {key: comparison[key] for key in
            ("different_snapshot_pairs", "state_differences", "biome_differences", "source_order_equal")}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeat-of", type=Path)
    args = parser.parse_args()
    directory, output = args.capture.resolve(), args.output.resolve()
    if not output.is_relative_to(ROOT / "target") or output.exists():
        parser.error("verification output must be a new target directory")
    output.mkdir(parents=True)
    try:
        for name in ("verify_admission.py", "admission_contract.py", "capture_admission.py"):
            (output / name).write_bytes((Path(__file__).parent / name).read_bytes())
        oracle = freeze_decoder(output)
        provenance, receipt = provenance_checks(directory)
        native = oracle.summarize(directory, verify=True)
        save(output / "baseline-verification.json", native)
        base = baseline_events(directory)
        with (directory / "admission-events.jsonl").open(encoding="utf-8") as stream:
            events = [json.loads(line) for line in stream]
        require(len(events) == receipt["events"], "sidecar event count differs from receipt")
        require(base["terminal"]["seq"] == receipt["baseline_terminal_seq"], "sidecar seal points to wrong baseline terminal")
        require(events[-1]["baseline_seq"] == receipt["last_sampled_baseline_seq"], "last sampled sequence differs from seal")
        analysis = analyze_sidecar(events, base["terminal"]["seq"], load(directory / "config.json")["requests"],
                                   base["stages"], base["stage_exits"])
        save(output / "admission-analysis.json", analysis)
        reference = Path(provenance["reference"])
        require(sha(reference / "provenance.json") == provenance["reference_provenance_sha256"], "reference provenance changed")
        reference_receipt = load(reference / "capture.json")
        require(sha(reference / "events.jsonl") == reference_receipt["events_sha256"], "reference trace changed")
        comparison = compare_snapshots(oracle, reference, directory, baseline_events(reference), base)
        save(output / "reference-comparison.json", comparison)
        if args.repeat_of:
            repeat = args.repeat_of.resolve()
            repeat_provenance, _ = provenance_checks(repeat)
            require(repeat_provenance["companion_sources"] == provenance["companion_sources"], "independent repeat companion sources differ")
            require(repeat_provenance["runner_sha256"] == provenance["runner_sha256"], "independent repeat runner differs")
            save(output / "repeat-comparison.json", compare_snapshots(oracle, repeat, directory, baseline_events(repeat), base))
        report = compact_report(directory, native, analysis, comparison)
        save(output / "report.json", report)
        save(output / "verification.json", {"schema": 1, "status": "pass", "capture": str(directory),
            "capture_provenance_sha256": sha(directory / "provenance.json"),
            "baseline_events_sha256": sha(directory / "events.jsonl"), "sidecar_events_sha256": sha(directory / "admission-events.jsonl"),
            "outputs": {file.name: sha(file) for file in sorted(output.iterdir()) if file.is_file()},
            "scope": "Baseline blobs and companion relations verified. Snapshot differences are preserved; no BCore or global equivalence claim."})
        print(json.dumps({key: report[key] for key in ("capture", "baseline_verified_blobs", "sidecar_events", "main_runnable_links",
              "executions_before_schedule_return", "scheduled_not_observed_executed", "snapshot_comparison")}, indent=2))
    except Exception as error:
        save(output / "failure.json", {"type": type(error).__name__, "error": str(error)})
        raise


if __name__ == "__main__":
    main()
