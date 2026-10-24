"""Export completed schema3 request/watch evidence; never execute or rescore a replay.

Inputs are explicit run_generation_check.py job receipts. Counts describe repeated
observations, not distinct chunks, implementation percentages or benchmarks.
"""
import argparse
from collections import Counter
import json
import math
from pathlib import Path
import re
import sys

import test_timings

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts/native-generation-reference"))
import oracle
import replay_bcore

METADATA = ("structures", "block_entity_payloads", "block_entity_packing", "saved_ticks",
            "postprocessing", "chunk_lifecycle", "save_boundary")
MAPS = {"worldgen_heightmaps": oracle.WG_MAPS, "runtime_heightmaps": oracle.RUNTIME_MAPS}
BUILD_ID = ("binary_sha256", "source_zip_sha256", "build_command", "rustc",
            "feature_trace_version", "binary_contract")
MEASURED_FIELDS = {"presence", "states", "biomes", "geometry", "serialized_light",
                   *("serialized_light/" + name for name in ("geometry", "presence", "length", "bytes")),
                   *("generated_metadata/" + name for name in METADATA),
                   *(group + "/" + name + "/" + kind for group, names in MAPS.items()
                     for name in names for kind in ("presence", "values"))}
SNAPSHOT_KEYS = {"pos", "snapshot_origin", "presence", "native_nbt", "native_extra", "scope",
                 "scored_matches", "matches", "unscored_fields", "content", "states", "biomes",
                 "geometry", "worldgen_heightmaps", "runtime_heightmaps", "serialized_light",
                 "generated_metadata", "native_effect_counts"}
REQUEST_KEYS = {"request", "spec", "native_event", "watch", "source_order", "requested_scored_matches",
                "bcore_complete", "bcore_coverage_status", "bcore_stages", "bcore_feature_sources",
                "unscored_row_fields"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def read(path):
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)


def resolve(value):
    require(isinstance(value, (str, Path)) and bool(str(value)), "missing evidence path")
    return (ROOT / value).resolve()


def receipt(path):
    return {"path": path.resolve().relative_to(ROOT).as_posix(), "sha256": oracle.sha(path)}


def count(value):
    require(type(value) is int and value >= 0, "counts must be nonnegative integers")
    return value


def job(path):
    path = resolve(path)
    require(path.parent == ROOT / "target/generation-checks" and path.suffix == ".json",
            "expected a root generation-checks job JSON")
    record = read(path)
    require(record.get("job") == path.stem and record.get("state") == "finished"
            and type(record.get("exit_code")) is int and record["exit_code"] == 0,
            "only a finished, exit-zero queue job can be exported")
    require(resolve(record.get("workdir")) == ROOT, "job must run in the root workspace")
    require(isinstance(record.get("command"), list) and record["command"]
            and all(isinstance(arg, str) and arg for arg in record["command"]), "invalid job command")
    times = [record.get(key) for key in ("started", "execution_started", "finished")]
    require(all(type(t) in (int, float) and math.isfinite(t) for t in times)
            and times == sorted(times), "missing or unordered queue execution timestamps")
    return record, {"job": record["job"], "command": record["command"], "exit_code": 0,
                    "status": receipt(path), "log": receipt(path.with_suffix(".log"))}


def options(tokens, flags=(), values=()):
    """Accept only the recorded command's supported options, without abbreviations."""
    result = {}
    tokens = iter(tokens)
    for token in tokens:
        key, equals, value = token.partition("=")
        require(key not in result, f"duplicate command option: {key}")
        if key in flags:
            require(not equals, f"unexpected value for {key}")
            result[key] = True
        else:
            require(key in values, f"unsupported command argument: {token}")
            value = value if equals else next(tokens, None)
            require(isinstance(value, str) and bool(value), f"missing value for {key}")
            result[key] = value
    return result


def test_command(command):
    require(Path(command[0]).name.lower() in ("cargo", "cargo.exe"), "expected cargo test")
    tail = command[2:] if len(command) > 1 and command[1].startswith("+") else command[1:]
    require(tail and tail[0] == "test", "expected cargo test")
    split = tail.index("--") if "--" in tail else len(tail)
    cargo = options(tail[1:split], ("--workspace", "--release", "--locked", "--no-fail-fast"),
                    ("-j", "--jobs"))
    require(cargo.get("--workspace") and cargo.get("--release"), "expected root workspace release tests")
    harness = options(tail[split + 1:], ("--report-time",), ("-Z", "--format", "--test-threads"))
    require(harness.get("-Z", "unstable-options") == "unstable-options"
            and harness.get("--format", "pretty") in ("json", "pretty", "terse"), "unsupported test harness")
    for value in (cargo.get("-j", "1"), cargo.get("--jobs", "1"), harness.get("--test-threads", "1")):
        require(value.isdecimal() and int(value) > 0, "invalid test concurrency")


def tests(path):
    record, evidence = job(path)
    test_command(record["command"])
    path = resolve(path)
    log = path.with_suffix(".log")
    summary = test_timings.summarize(log, path)
    require(summary["target_count"] > 0, "workspace log lacks completed target results")
    require(all(t["outcome"] == "ok" and t["failed"] == 0 for t in summary["targets"])
            and all(t["outcome"] != "failed" for t in summary["tests"]), "workspace tests failed")
    # summarize supplies the totals; this checks that a trailing harness wasn't lost.
    pending, expected, completed = False, None, 0
    for line in log.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line.startswith(("Running ", "Doc-tests ")):
            require(not pending, "missing target result before next harness")
            pending = True
        event = json.loads(line, object_pairs_hook=unique_object) if line.startswith("{") else {}
        if event.get("type") == "suite" and event.get("event") == "started":
            require(expected is None, "missing JSON suite result")
            expected, pending = count(event["test_count"]), True
        text = test_timings.SUMMARY.fullmatch(line)
        if text or (event.get("type") == "suite" and event.get("event") in ("ok", "failed")):
            require(pending, "target result lacks a harness start")
            numbers = ([int(text[i]) for i in range(2, 7)] if text else
                       [event[k] for k in ("passed", "failed", "ignored", "measured", "filtered_out")])
            for number in numbers:
                count(number)
            require(expected is None or sum(numbers[:4]) == expected, "incomplete JSON suite totals")
            pending, expected, completed = False, None, completed + 1
    require(not pending and completed == summary["target_count"], "missing final target result")
    require(all(math.isfinite(t["seconds"]) and t["seconds"] >= 0 for t in summary["targets"]),
            "invalid suite timings")
    per_test = summary.pop("tests")
    summary["per_test_timings"] = sum(t["seconds"] is not None for t in per_test)
    summary["ignored_tests"] = [t for t in per_test if t["outcome"] == "ignored"]
    require(summary["log_sha256"] == evidence["log"]["sha256"], "test log changed while reading")
    return {**summary, **evidence}


def replay_command(command):
    require(re.fullmatch(r"python(?:3(?:\.\d+)?)?(?:\.exe)?", Path(command[0]).name, re.I),
            "expected a direct Python replay_bcore.py invocation")
    tail = command[1:]
    while tail and tail[0] in ("-B", "-u"):
        tail = tail[1:]
    require(len(tail) >= 2 and resolve(tail[0]) == ROOT / replay_bcore.OBSERVER / "replay_bcore.py",
            "expected the current root replay_bcore.py script")
    args = options(tail[2:], values=("--output", "--reuse-build", "--timeout", "--trace-feature"))
    require("--output" in args, "replay command lacks --output")
    timeout = args.get("--timeout", "1200")
    require(timeout.isdecimal() and int(timeout) > 0, "invalid replay timeout")
    capture, out = resolve(tail[1]), resolve(args["--output"])
    require(out.is_relative_to(ROOT / "target"), "replay output must be under root target/")
    return capture, out, args


def build_chain(directory, trace_feature=False):
    chain, seen, identity = [], set(), None
    while True:
        require(directory not in seen, "cyclic reused build provenance")
        seen.add(directory)
        prior = replay_bcore.validate_reuse_build(directory, replay_bcore.HISTORY_PROGRAM, trace_feature)
        current = {key: prior[key] for key in BUILD_ID}
        require(identity is None or oracle.same_value(identity, current), "mixed source/binary build identity")
        identity = current
        files = ("provenance.json", "source.zip", "tools.zip", "replay.json", replay_bcore.HISTORY_PROGRAM + ".exe")
        chain.append({"files": {name: receipt(directory / name) for name in files},
                      "runner_sha256": prior["runner_sha256"], "comparator_sha256": prior["comparator_sha256"]})
        if (directory / "build.log").is_file():
            chain[-1]["files"]["build.log"] = receipt(directory / "build.log")
        if "reused_build" not in prior:
            return identity, chain
        directory = resolve(prior["reused_build"])
        require(oracle.sha(directory / "provenance.json") == prior.get("reused_provenance_sha256"),
                "reused build provenance digest mismatch")


def unscored(reason):
    return {"scored": False, "matches": None, "mismatches": None, "reason": reason}


def score(value, vector=False):
    require(isinstance(value, dict) and type(value.get("scored")) is bool, "missing comparison field coverage")
    if not value["scored"]:
        require(value.get("matches") is None, "unscored field claims a match")
        result = unscored(value.get("reason", "observation unavailable"))
    else:
        differences = count(value.get("mismatches"))
        require(value.get("matches") is (differences == 0), "inconsistent comparison match flag")
        require(isinstance(value.get("differences"), list) and len(value["differences"]) == differences,
                "incomplete comparison differences")
        result = {"scored": True, "matches": value["matches"], "mismatches": differences}
        if vector:
            total = count(value.get("total"))
            lengths = [count(value.get(k, total)) for k in ("expected_length", "actual_length")]
            require(differences <= total and max(lengths) == total, "inconsistent observation counts")
            result.update(total=total, expected_length=lengths[0], actual_length=lengths[1])
    if "native_counts" in value:
        result["native_counts"] = value["native_counts"]
    return result


def inventory(fields):
    require(isinstance(fields, list) and all(isinstance(f, dict) and f.get("scored") is False
            and isinstance(f.get("path"), list) and f["path"] and isinstance(f.get("reason"), str)
            for f in fields), "missing or invalid unscored inventory")
    return list(fields)


def extensions(data, known, path=()):
    return [{"path": ["comparison", *path, key], "scored": False,
             "reason": "exporter does not interpret this field", "value": data[key]}
            for key in sorted(data.keys() - set(known))]


def counted(total, differences):
    return {"scored": True, "matches": differences == 0, "total": total, "mismatches": differences}


def heightmaps(row, actual, fields, unknown):
    for group, names in MAPS.items():
        maps = row[group] if group == "worldgen_heightmaps" else row[group]["maps"]
        require(set(names) <= maps.keys(), "missing heightmap comparison")
        unknown.extend(extensions(maps, names, (group,)))
        for name in names:
            value = maps[name]
            prefix = group + "/" + name
            require(type(value.get("scored")) is bool, "missing heightmap coverage")
            if not value["scored"]:
                fields[prefix + "/presence"] = score(value)
                fields[prefix + "/values"] = unscored(value.get("reason", "heightmap observation unavailable"))
                continue
            present = [value.get(k) for k in ("native_present", "bcore_present")]
            require(all(type(p) is bool for p in present)
                    and value.get("presence_matches") is (present[0] == present[1]), "invalid heightmap presence")
            require(group in actual and (actual[group] is None or isinstance(actual[group], dict))
                    and present[1] == (name in (actual[group] or {})), "heightmap presence/output mismatch")
            fields[prefix + "/presence"] = counted(1, int(present[0] != present[1])) | {
                "native_present": present[0], "bcore_present": present[1]}
            values = score(value.get("values"), True) if all(present) else unscored("one or both maps absent")
            if all(present):
                require(isinstance(actual[group][name], list) and values.get("actual_length") == len(actual[group][name]),
                        "heightmap value/output count mismatch")
            fields[prefix + "/values"] = values
            require(value.get("matches") is (values["matches"] if all(present) else present[0] == present[1]),
                    "inconsistent heightmap match flag")
        if group == "runtime_heightmaps":
            scored = all(maps[name]["scored"] for name in names)
            require(row[group].get("scored") is scored and row[group].get("matches") is
                    (all(maps[name]["matches"] for name in names) if scored else None), "invalid runtime map coverage")


def light_fields(light, actual, fields):
    require(type(light.get("scored")) is bool and type(light.get("native_is_light_on")) is bool
            and type(light.get("bcore_snapshot_available")) is bool, "missing light coverage")
    require(light["scored"] is light["bcore_snapshot_available"]
            and light["bcore_snapshot_available"] is (actual.get("light") is not None), "light coverage/output mismatch")
    fields["serialized_light"] = {k: light[k] for k in ("scored", "native_is_light_on", "bcore_snapshot_available")}
    fields["serialized_light"]["matches"] = light.get("matches")
    if not light["scored"]:
        require(light.get("matches") is None, "unscored light claims a match")
        return
    geometry = score(light.get("geometry"))
    require(light.get("geometry_matches") is geometry["matches"], "inconsistent light geometry")
    layers = light["layers"]
    require(isinstance(layers, list), "missing light layers")
    byte_scores, presence, seen = [], 0, set()
    for layer in layers:
        key = (layer["y"], layer["kind"])
        require(type(key[0]) is int and key[1] in ("sky", "block") and key not in seen, "invalid light layer")
        seen.add(key)
        pair = [layer.get(k) for k in ("expected_present", "actual_present")]
        require(all(type(p) is bool for p in pair) and layer.get("presence_matches") is (pair[0] == pair[1]),
                "invalid light presence")
        presence += pair[0] != pair[1]
        if all(pair):
            byte_scores.append(score(layer.get("bytes"), True))
    lengths = sum(s["expected_length"] != s["actual_length"] for s in byte_scores)
    differences = sum(s["mismatches"] for s in byte_scores)
    require(all(type(light.get(k)) is int and light[k] == n for k, n in
                (("presence_mismatches", presence), ("length_mismatches", lengths), ("byte_mismatches", differences))),
            "inconsistent light counts")
    require(light.get("matches") is (geometry["matches"] and not (presence or lengths or differences)),
            "inconsistent light match flag")
    fields.update({"serialized_light/geometry": geometry,
                   "serialized_light/presence": counted(len(layers), presence),
                   "serialized_light/length": counted(len(byte_scores), lengths) if byte_scores else unscored("no common light layers"),
                   "serialized_light/bytes": counted(sum(s["total"] for s in byte_scores), differences)
                   if byte_scores else unscored("no common light layers")})


def snapshot(row, native, actual, origin, requested=False):
    oracle.validate_snapshot(native, actual["pos"], origin, native=True)
    require(oracle.position_key(row.get("pos")) == oracle.position_key(actual["pos"])
            and row["snapshot_origin"] == origin, "comparison snapshot position/origin mismatch")
    for kind in ("nbt", "extra"):
        require("native_" + kind in row and row["native_" + kind] == native.get(kind), "native snapshot reference mismatch")
    present = [not s.get("absent", False) for s in (native, actual)]
    fields = {"presence": score(row.get("presence")) | {"native_present": present[0], "bcore_present": present[1]}}
    require(fields["presence"]["mismatches"] == int(present[0] != present[1]), "snapshot presence mismatch")
    if present[0] != present[1]:
        require(row["presence"]["differences"][0].get("expected") is present[0]
                and row["presence"]["differences"][0].get("actual") is present[1], "snapshot presence direction mismatch")
    unknown = inventory(row.get("unscored_fields"))
    unknown.extend(extensions(row, SNAPSHOT_KEYS | (REQUEST_KEYS if requested else set())))
    if all(present):
        require(row["scope"] == "scored snapshot fields", "invalid content comparison scope")
        for name in ("states", "biomes", "geometry"):
            fields[name] = score(row.get(name), name != "geometry")
            if name != "geometry":
                require(fields[name].get("actual_length") == len(actual[name]), "comparison/output observation count mismatch")
        heightmaps(row, actual, fields, unknown)
        light_fields(row["serialized_light"], actual, fields)
        metadata = row["generated_metadata"]
        require(set(METADATA) <= metadata.keys(), "missing logical metadata coverage")
        unknown.extend(extensions(metadata, METADATA, ("generated_metadata",)))
        for name in METADATA:
            value = fields["generated_metadata/" + name] = score(metadata[name])
            if name in ("chunk_lifecycle", "save_boundary") and value["scored"]:
                value["differences"] = metadata[name]["differences"]
        effects = row["native_effect_counts"]
        for value in effects.values():
            count(value)
    else:
        require(row["scope"] == "presence" and row.get("content", {}).get("scored") is False,
                "absent snapshot requires explicit presence-only coverage")
        require(not {"states", "biomes", "geometry", "generated_metadata", "serialized_light"} & row.keys(),
                "absent boundary contains content comparisons")
        fields["content"] = score(row["content"])
        effects = None
    for name in MEASURED_FIELDS - fields.keys():
        fields[name] = unscored("outside snapshot scope" if not all(present) else "observation unavailable")
    matching = all(value["matches"] is True for value in fields.values() if value["scored"])
    require(row.get("requested_scored_matches" if requested else "scored_matches") is matching,
            "snapshot scored match flag differs from fields")
    if not requested:
        expected = False if not matching else None if all(present) or unknown else True
        require(row.get("matches") is expected, "invalid watch parity claim")
    return {"pos": row["pos"], "snapshot_origin": origin, "scope": row["scope"], "fields": fields,
            "scored_matches": matching, "matches": False if not matching else None,
            "native_nbt": row["native_nbt"], "native_extra": row["native_extra"],
            "native_effect_counts": effects, "unscored_fields": unknown}


def score_totals(values):
    scored = [v for v in values if v["scored"]]
    result = {"count": len(values), "scored": len(scored), "unscored": len(values) - len(scored),
              "matches": sum(v["matches"] is True for v in scored),
              "mismatching": sum(v["matches"] is False for v in scored)}
    for source, name in (("mismatches", "differences"), ("total", "observations"),
                         ("expected_length", "native_observations"), ("actual_length", "bcore_observations")):
        numbers = [v[source] for v in scored if v.get(source) is not None]
        result[name] = sum(numbers) if numbers else None
    return result


def snapshot_totals(rows):
    names = MEASURED_FIELDS | {name for row in rows for name in row["fields"]}
    return {"snapshots": len(rows), "scored_matching_snapshots": sum(r["scored_matches"] for r in rows),
            "scopes": dict(Counter(r["scope"] for r in rows)),
            "native_present": sum(r["fields"]["presence"]["native_present"] for r in rows),
            "bcore_present": sum(r["fields"]["presence"]["bcore_present"] for r in rows),
            "unscored_fields": sum(len(r["unscored_fields"]) for r in rows),
            "fields": {name: score_totals([r["fields"].get(name, unscored("outside snapshot scope")) for r in rows])
                       for name in sorted(names)}}


def totals(rows):
    stages = [stage for row in rows for stage in row["bcore"]["stages"] or []]
    return {"requests": len(rows), "requested": snapshot_totals([r["requested"] for r in rows]),
            "watch": snapshot_totals([w for r in rows for w in r["watch"]]),
            "source_order": score_totals([r["source_order"] for r in rows]),
            "unscored_row_fields": sum(len(r["unscored_row_fields"]) for r in rows),
            "bcore": {"complete_requests": sum(r["bcore"]["complete"] is True for r in rows),
                      "incomplete_requests": sum(r["bcore"]["complete"] is False for r in rows),
                      "absent_requests": sum(r["bcore"]["complete"] is None for r in rows),
                      "partial_requests": sum(any(s["state"] == "Partial" for s in r["bcore"]["stages"] or []) for r in rows),
                      "stage_states": {status: dict(Counter(s["state"] for s in stages if s["status"] == status))
                                       for status in sorted({s["status"] for s in stages})}}}


def replay(path):
    record, evidence = job(path)
    capture, out, args = replay_command(record["command"])
    build, chain = build_chain(out, "--trace-feature" in args)
    prior, plan, diff = (read(out / name) for name in ("provenance.json", "replay.json", "comparison.json"))
    require(type(diff.get("schema")) is int and diff["schema"] == oracle.REPLAY_SCHEMA
            and diff.get("observation_contract") == oracle.OBSERVATION_CONTRACT, "comparison requires schema 3")
    require(resolve(diff.get("capture")) == capture, "comparison capture differs from command")
    require(("--reuse-build" in args) == ("reused_build" in prior), "reuse command/provenance mismatch")
    if "--reuse-build" in args:
        require(resolve(args["--reuse-build"]) == resolve(prior["reused_build"]), "wrong reused build directory")
    run = prior.get("run_command")
    require(isinstance(run, list) and len(run) == 3 and [resolve(p) for p in run] ==
            [out / (replay_bcore.HISTORY_PROGRAM + ".exe"), out / "replay.json", out / "chunks.jsonl"],
            "recorded run command/output directory mismatch")
    require(prior.get("result_sha256") == diff.get("rust_output_sha256") == oracle.sha(out / "chunks.jsonl"),
            "comparison/chunks/provenance digest mismatch")
    require(diff.get("comparator_sha256") == prior["comparator_sha256"], "comparison/invocation comparator mismatch")
    identity = oracle.native_input_identity(capture)
    require(all(diff.get(k) == prior.get(k) == plan.get(k) == v for k, v in identity.items()),
            "comparison/native input provenance mismatch")
    events = oracle.load_events(capture)
    native_plan = oracle.plan_from_events(read(capture / "config.json"), events, identity)
    require(all(oracle.same_value(plan.get(k), v) for k, v in native_plan.items()), "replay plan/native history mismatch")
    require(("--trace-feature" in args) == ("feature_trace" in plan), "feature trace command/plan mismatch")
    if "--trace-feature" in args:
        x, z, feature = args["--trace-feature"].split(",", 2)
        require(plan["feature_trace"] == {"source": [int(x), int(z)], "feature": feature}
                and oracle.sha(out / "feature_traces.jsonl") == prior.get("feature_trace_sha256"), "trace receipt mismatch")
        evidence["feature_trace"] = receipt(out / "feature_traces.jsonl")
    with (out / "chunks.jsonl").open(encoding="utf-8") as stream:
        actual = [json.loads(line, object_pairs_hook=unique_object) for line in stream]
    oracle.validate_replay_rows(actual, plan)
    actual = actual[len(plan["bootstrap_requests"]):]
    native = [e for e in events if e["event"] == "request_exit"]
    require(isinstance(diff.get("requests"), list) and len(diff["requests"]) == len(actual) and actual,
            "missing or incomplete comparison requests")
    rows, blobs = [], {}
    for row, raw, event in zip(diff["requests"], actual, native):
        require(type(row.get("request")) is int and row["request"] == raw["request"]
                and type(row.get("native_event")) is int and row["native_event"] == event["seq"]
                and oracle.same_value(row.get("spec"), raw["spec"]), "comparison request history/order mismatch")
        oracle.validate_watches(event.get("watch"), plan["watch"], native=True)
        require(isinstance(row.get("watch"), list) and len(row["watch"]) == len(plan["watch"]), "missing comparison watches")
        coverage = {key: raw[key] for key in ("complete", "coverage_status", "stages", "feature_sources")}
        require(all(oracle.same_value(row.get("bcore_" + k), v) for k, v in coverage.items()), "comparison coverage/output mismatch")
        requested = snapshot(row, event["snapshot"], raw["snapshot"], "returned_object", True)
        watches = [snapshot(w, n, a, "holder_latest") for w, n, a in zip(row["watch"], event["watch"], raw["watch"])]
        sequence = score(row.get("source_order"), True)
        require(sequence.get("actual_length") == len(raw["feature_sources"]), "source order/output count mismatch")
        native_sources = sum(e["event"] == "stage_enter" and e["status"] == "minecraft:features"
                             and e["seq"] < event["seq"] for e in events)
        require(sequence.get("expected_length") == native_sources, "source order/native count mismatch")
        matching = requested["scored_matches"] and sequence["matches"] and all(w["scored_matches"] for w in watches)
        require(row.get("scored_matches") is matching and row.get("matches") is (None if matching else False),
                "request match flag erases differences or claims full parity")
        rows.append({"request": row["request"], "spec": row["spec"], "native_event": row["native_event"],
                     "requested": requested, "watch": watches, "source_order": sequence, "bcore": coverage,
                     "scored_matches": matching, "matches": row["matches"],
                     "unscored_row_fields": inventory(row.get("unscored_row_fields"))})
        for item in [requested, *watches]:
            for name in (item["native_nbt"], item["native_extra"]):
                if name is not None and name not in blobs:
                    oracle.blob(capture, name)  # Validate bytes only; no NBT decoding or expected-output calculation.
                    blobs[name] = receipt(capture / "blobs" / name)
    matching = all(r["scored_matches"] for r in rows)
    require(diff.get("scored_matches") is matching and diff.get("matches") is (None if matching else False),
            "history match flag erases differences or claims full parity")
    logged = read(resolve(path).with_suffix(".log"))
    require(resolve(logged.get("output")) == out and oracle.same_value(logged.get("requests"), replay_bcore.comparison_summary(diff)),
            "completed job log differs from replay output/comparison")
    evidence["replay"] = {name: receipt(out / name) for name in ("comparison.json", "chunks.jsonl", "run.log")}
    evidence["build_chain"] = chain
    evidence["native"] = {name: receipt(capture / name) for name in (*oracle.NATIVE_INPUT_FILES.values(), "capture.json")}
    evidence["native_blobs"] = blobs
    return {"name": capture.name, "capture": capture.relative_to(ROOT).as_posix(), "build": build,
            "native_input_identity": identity, "comparator_sha256": diff["comparator_sha256"],
            "run_command": run, "evidence": evidence, "seed": plan["seed"], "game_time": plan["game_time"],
            "bootstrap": plan["bootstrap"], "bootstrap_calls": len(plan["bootstrap_requests"]),
            "watch_positions": plan["watch"], "requests": rows, "totals": totals(rows),
            "scored_matches": matching, "matches": diff["matches"], "limitations": diff["limitations"],
            "unscored_fields": extensions(diff, {"schema", "observation_contract", "capture", *identity,
                "comparator_sha256", "rust_output_sha256", "requests", "scored_matches", "matches", "limitations"})}


def export(tests_path, replay_jobs, output):
    output = resolve(output)
    require(output.is_relative_to(ROOT / "docs/metrics") and output.suffix == ".json", "output must be a new JSON under docs/metrics")
    require(not output.exists(), "output already exists")
    require(replay_jobs, "at least one explicit replay job is required")
    test_result = tests(tests_path)
    histories = sorted((replay(path) for path in replay_jobs), key=lambda h: h["capture"])
    require(len({h["capture"] for h in histories}) == len(histories), "duplicate replay history")
    require(all(oracle.same_value(h["build"], histories[0]["build"]) for h in histories), "mixed source/binary build identity")
    rows = [r for h in histories for r in h["requests"]]
    result = {"schema": 1, "replay_schema": oracle.REPLAY_SCHEMA, "observation_contract": oracle.OBSERVATION_CONTRACT,
              "exporter": receipt(Path(__file__)), "tests": test_result, "histories": histories,
              "totals": {"histories": len(histories), "bootstrap_calls": sum(h["bootstrap_calls"] for h in histories), **totals(rows)},
              "scored_matches": all(r["scored_matches"] for r in rows),
              "matches": None if all(r["scored_matches"] for r in rows) else False,
              "scope": "Requested returned-object and ordered holder-watch snapshots are separate, repeated observations, including air and length tails. They must not be added into a distinct-chunk count.",
              "limitations": ["Only recorded scored fields contribute to scored_matches; matches=null is not full parity. BCore implementation coverage is reported independently.",
                              "Absent boundaries and unsupported fields remain unscored for content. Raw differences and unknown fields remain available in hashed evidence.",
                              "Logical NBT comparisons do not score numeric tag widths or compound wire order. Serialized light does not score isLightOn or lazy-zero versus absent storage.",
                              "Queue/build/harness timings are validation receipts, not benchmark results. No historical checkpoint totals are carried forward."]}
    rendered = json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + "\n"
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        stream.write(rendered)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tests", type=Path, required=True)
    parser.add_argument("--replay-job", action="append", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = export(args.tests, args.replay_job, args.output)
    print(json.dumps({"output": str(args.output), "tests": result["tests"]["totals"],
                      "histories": result["totals"]["histories"], "requests": result["totals"]["requests"]}, sort_keys=True))


if __name__ == "__main__":
    main()
