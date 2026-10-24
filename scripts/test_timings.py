"""Separate Cargo build time from libtest runtime in saved text or JSON logs.

For per-test timings, use a nightly test harness with:
    cargo test ... -- -Z unstable-options --format json --report-time

This command only reads evidence. It does not run tests or alter their workload.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re


SUMMARY = re.compile(
    r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out; finished in ([\d.]+)s"
)
FINISHED = re.compile(r"Finished .+ target\(s\) in (.+)")
DURATION = re.compile(r"([\d.]+)(ms|s|m|h)")
UNIT_SECONDS = {"ms": 0.001, "s": 1, "m": 60, "h": 3600}


def summarize(log, status=None):
    raw = log.read_bytes()
    builds, targets, tests = [], [], []
    target = "unlabelled harness"
    for line in raw.decode("utf-8", errors="replace").splitlines():
        line = line.strip()
        if line.startswith(("Running ", "Doc-tests ")):
            target = line
        finished = FINISHED.search(line)
        if finished:
            builds.append(sum(float(n) * UNIT_SECONDS[u]
                              for n, u in DURATION.findall(finished[1])))
        summary = SUMMARY.fullmatch(line)
        if summary:
            targets.append(dict(target=target, outcome=summary[1],
                                passed=int(summary[2]), failed=int(summary[3]),
                                ignored=int(summary[4]), measured=int(summary[5]),
                                filtered_out=int(summary[6]), seconds=float(summary[7])))
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "test" and event.get("event") in ("ok", "failed", "ignored"):
            tests.append(dict(target=target, name=event["name"], outcome=event["event"],
                              seconds=event.get("exec_time")))
        if event.get("type") == "suite" and event.get("event") in ("ok", "failed"):
            targets.append(dict(target=target, outcome=event["event"],
                                passed=event["passed"], failed=event["failed"],
                                ignored=event["ignored"], measured=event["measured"],
                                filtered_out=event["filtered_out"], seconds=event["exec_time"]))
    result = {
        "log": str(log.resolve()),
        "log_sha256": hashlib.sha256(raw).hexdigest(),
        "build_seconds": builds,
        "target_count": len(targets),
        "harness_seconds": sum(t["seconds"] for t in targets),
        "totals": {key: sum(t[key] for t in targets)
                   for key in ("passed", "failed", "ignored", "measured", "filtered_out")},
        "targets": sorted(targets, key=lambda t: t["seconds"], reverse=True),
        "tests": sorted(tests, key=lambda t: t["seconds"] or 0, reverse=True),
    }
    if status:
        record = json.loads(status.read_text(encoding="utf-8"))
        result["job"] = record["job"]
        result["command"] = record["command"]
        result["exit_code"] = record.get("exit_code")
        if "execution_started" in record:
            result["queue_wait_seconds"] = record["execution_started"] - record["started"]
        if "execution_started" in record and "finished" in record:
            elapsed = record["finished"] - record["execution_started"]
            result["execution_seconds"] = elapsed
            result["unattributed_seconds"] = elapsed - sum(builds) - result["harness_seconds"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--status", type=Path, help="optional run_check.py job JSON")
    args = parser.parse_args()
    print(json.dumps(summarize(args.log, args.status), indent=2))


if __name__ == "__main__":
    main()
