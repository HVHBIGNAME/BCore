"""Publish compact metrics from completed tests and immutable native replay evidence.

Raw captures remain in target/. This export keeps counts, request specifications,
known differences and source hashes suitable for review on GitHub. It never
recomputes native expected output or turns a partial stage into completion.
"""
import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import re
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def published_sha(path):
    # .gitattributes publishes JSON with LF. Locally generated older checkpoints
    # can still have CRLF, so reference their published bytes rather than a
    # Windows-only working copy. Raw replay/artifact hashes above stay byte-exact.
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def relative(path):
    return path.resolve().relative_to(ROOT).as_posix()


def replay(directory):
    path = directory / "comparison.json"
    data = read(path)
    provenance = read(directory / "provenance.json")
    build = provenance
    if "reused_build" in provenance:
        build_path = Path(provenance["reused_build"]) / "provenance.json"
        if sha(build_path) != provenance["reused_provenance_sha256"]:
            raise ValueError("reused build provenance changed")
        build = read(build_path)
        if build["binary_sha256"] != provenance["binary_sha256"]:
            raise ValueError("reused binary identity changed")
    if build.get("build_returncode") != 0:
        raise ValueError(f"replay lacks a successful build: {directory}")
    build_directory = Path(provenance.get("reused_build", directory))
    source_archive = build_directory / "source.zip"
    if sha(source_archive) != build["source_zip_sha256"]:
        raise ValueError("frozen source archive differs from recorded build")
    with zipfile.ZipFile(source_archive) as archive:
        actual = {name: hashlib.sha256(archive.read(name)).hexdigest() for name in archive.namelist()}
    if actual != build["source_sha256"]:
        raise ValueError("frozen source manifest differs from archive contents")
    if sha(Path(provenance["run_command"][0])) != provenance["binary_sha256"]:
        raise ValueError("frozen replay executable differs from recorded binary")
    if sha(directory / "chunks.jsonl") != data["rust_output_sha256"]:
        raise ValueError("replay output differs from scored comparison")
    if data["rust_output_sha256"] != provenance["result_sha256"]:
        raise ValueError("scored output differs from the recorded replay result")
    if data["native_provenance_sha256"] != provenance["native_provenance_sha256"]:
        raise ValueError("comparison and replay refer to different native captures")
    capture = Path(data["capture"])
    if not capture.is_absolute():
        # Worker captures record paths relative to their own worktree, not main.
        worktree = next(parent for parent in directory.resolve().parents if (parent / ".git").exists())
        capture = worktree / capture
    if sha(capture / "provenance.json") != data["native_provenance_sha256"]:
        raise ValueError("native capture provenance differs from the scored comparison")
    result = {"name": Path(data["capture"].replace("\\", "/")).name,
              "comparison": relative(path), "comparison_sha256": sha(path),
              "binary_sha256": provenance["binary_sha256"],
              "source_zip_sha256": build["source_zip_sha256"],
              "native_provenance_sha256": data["native_provenance_sha256"],
              "comparator_sha256": data["comparator_sha256"], "requests": []}
    for row in data["requests"]:
        light = row["serialized_light"]
        meta = row["generated_metadata"]
        result["requests"].append({
            "chunk": [int(v) for v in row["spec"]["pos"]], "status": row["spec"]["status"],
            "complete": row["bcore_complete"],
            "states": {k: row["states"][k] for k in ("mismatches", "total")},
            "biomes": {k: row["biomes"][k] for k in ("mismatches", "total")},
            "source_order_differences": row["source_order"]["mismatches"],
            "light_scored": light["scored"],
            "light_matches": light.get("matches"),
            "metadata": {k: {name: item[name] for name in ("scored", "mismatches") if name in item}
                         for k, item in meta.items()},
            "wg_presence_differences": sum(not v["presence_matches"] for v in row["worldgen_heightmaps"].values()),
        })
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tests", type=Path, required=True)
    parser.add_argument("--wave", type=Path, required=True)
    parser.add_argument("--replay", action="append", type=Path, default=[])
    parser.add_argument("--before", action="append", type=Path, default=[])
    parser.add_argument(
        "--carry-before-after",
        type=Path,
        help="reuse the before/after rows of an already published checkpoint whose frozen replays were pruned",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    status = read(args.tests)
    if status.get("state") != "finished" or status.get("exit_code") != 0:
        raise ValueError("only a completed successful workspace run can be published")
    if status["command"][:2] != ["cargo", "test"] or "--workspace" not in status["command"]:
        raise ValueError("the test checkpoint must be a workspace test run")
    log = args.tests.with_suffix(".log")
    text = log.read_text(encoding="utf-8", errors="replace")
    summaries = re.findall(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored", text)
    if not summaries or any(row[0] != "ok" or int(row[2]) for row in summaries):
        raise ValueError("workspace log contains missing/failed target results")
    tests = {"job": status["job"], "command": status["command"], "exit_code": 0,
             "target_summaries": len(summaries), "passed": sum(int(v[1]) for v in summaries),
             "failed": 0, "ignored": sum(int(v[3]) for v in summaries),
             "log_sha256": sha(log), "status_sha256": sha(args.tests),
             "ignored_tests": re.findall(r"^test (.+?) \.\.\. ignored.*$", text, re.MULTILINE)}
    wave = read(args.wave / "status.json")
    if wave["state"] != "finished" or any(case["exit_code"] for case in wave["cases"]):
        raise ValueError("wave contains unfinished or failed replays")
    histories = [replay(args.wave / Path(case["capture"]).name) for case in wave["cases"]]
    histories.extend(map(replay, args.replay))
    if len({h["name"] for h in histories}) != len(histories):
        raise ValueError("duplicate replay history")
    if len({h["binary_sha256"] for h in histories}) != 1:
        raise ValueError("the combined checkpoint must use one frozen executable")
    rows = [row for history in histories for row in history["requests"]]
    totals = {"histories": len(histories), "requests": len(rows),
              "complete_requests": sum(r["complete"] for r in rows),
              "state_observations": sum(r["states"]["total"] for r in rows),
              "state_differences": sum(r["states"]["mismatches"] for r in rows),
              "biome_observations": sum(r["biomes"]["total"] for r in rows),
              "biome_differences": sum(r["biomes"]["mismatches"] for r in rows),
              "source_order_differences": sum(r["source_order_differences"] for r in rows),
              "light_scored_requests": sum(r["light_scored"] for r in rows),
              "light_mismatching_requests": sum(r["light_scored"] and not r["light_matches"] for r in rows),
              "wg_presence_differences": sum(r["wg_presence_differences"] for r in rows),
              "metadata_field_differences": {name: sum(r["metadata"][name]["mismatches"] for r in rows)
                                             for name in ("structures", "block_entity_payloads", "postprocessing")}}
    before_after = []
    for directory in args.before:
        before = replay(directory)
        after = next(h for h in histories if h["name"] == before["name"])
        specs = lambda history: [(r["chunk"], r["status"]) for r in history["requests"]]
        if specs(before) != specs(after):
            raise ValueError("before/after must compare the same native request history")
        if before["native_provenance_sha256"] != after["native_provenance_sha256"]:
            raise ValueError("before/after must use the identical native capture")
        before_after.append({"history": before["name"], "requests": len(before["requests"]),
                             "before_state_differences": sum(r["states"]["mismatches"] for r in before["requests"]),
                             "after_state_differences": sum(r["states"]["mismatches"] for r in after["requests"]),
                             "before_comparison_sha256": before["comparison_sha256"],
                             "after_comparison_sha256": after["comparison_sha256"]})
    if args.carry_before_after:
        # The frozen replays behind these rows were pruned after publication, so
        # the rows are carried verbatim from the already published checkpoint
        # instead of being recomputed. Their recorded comparison hashes stay
        # historical; they must not be relabelled as this wave's comparisons.
        carried = read(args.carry_before_after)
        if not carried.get("before_after"):
            raise ValueError("carried checkpoint has no before/after rows")
        current = {h["name"]: h for h in histories}
        previous = {h["name"]: h for h in carried["histories"]}
        specs = lambda history: [(r["chunk"], r["status"]) for r in history["requests"]]
        for row in carried["before_after"]:
            name = row["history"]
            if name not in current or name not in previous:
                raise ValueError(f"carried history is absent from this wave: {row['history']}")
            if (current[name]["native_provenance_sha256"] != previous[name]["native_provenance_sha256"]
                    or specs(current[name]) != specs(previous[name])
                    or row["requests"] != len(current[name]["requests"])):
                raise ValueError(f"carried history has different native inputs: {name}")
            if row["after_state_differences"] != sum(r["states"]["mismatches"] for r in current[name]["requests"]):
                raise ValueError(f"carried historical result differs from current wave: {name}")
            if row["after_comparison_sha256"] != previous[name]["comparison_sha256"]:
                raise ValueError(f"carried historical comparison identity differs: {name}; use the original checkpoint")
        before_after = [{"history": row["history"], "requests": row["requests"],
                         "before_state_differences": row["before_state_differences"],
                         "after_state_differences": row["after_state_differences"],
                         "before_comparison_sha256": row["before_comparison_sha256"],
                         "after_comparison_sha256": row["after_comparison_sha256"],
                         "carried_from": relative(args.carry_before_after),
                         "carried_from_sha256": published_sha(args.carry_before_after),
                         "comparison_scope": "historical before/after, not recomputed for this wave"}
                        for row in carried["before_after"]]
        if args.before:
            raise ValueError("--before and --carry-before-after are exclusive")
    result = {"schema": 1, "published_date": dt.datetime.now(dt.timezone.utc).date().isoformat(),
              "minecraft": "26.1", "protocol": 775, "tests": tests, "totals": totals,
              "scope": "Matched bootstrap/request/source histories. Counts include repeated snapshots and air; they are not a percentage of all generator features implemented.",
              "remaining": ["live server clock/settings hookup for SPAWN", "FULL conversion and ticket-driven completion",
                            "WG heightmap lifetime after FULL", "tick-container lifecycle", "other structures/features",
                            "saved-holder hydration", "Nether/End pipelines"],
              "histories": histories, "before_after": before_after}
    if not args.output.resolve().is_relative_to(ROOT / "docs/metrics"):
        parser.error("published output must be under docs/metrics")
    with args.output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"output": relative(args.output), "tests": tests, "totals": totals,
                      "before_after": before_after}, indent=2))


if __name__ == "__main__":
    main()
