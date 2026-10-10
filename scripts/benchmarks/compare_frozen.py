"""Alternate two verified BCore executables without rebuilding either snapshot.

This supplementary same-session comparison is not a new Java measurement. Each
input must be a completed matched-output noise-fill-wg-v2 run with its frozen
binary and source archive still available. Use the project's heavy-work queue.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import statistics
import time

from provenance import controlled_environment, freeze_copy, sha, verify_files, write_json
from run import CHUNKS, ROOT, compare_contract, execute, validate_sample


def load_run(path):
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("verified_matching_outputs") is not True:
        raise ValueError(f"input is not a completed matched-output run: {path}")
    return data


def compare(before_path, after_path, output):
    before, after = load_run(before_path), load_run(after_path)
    compare_contract(before, after)
    if before["fingerprints"] != after["fingerprints"]:
        raise ValueError("input fingerprints differ")
    env, environment = controlled_environment()
    if environment != after["provenance"]["environment"]:
        raise ValueError("current environment policy differs from recorded runs")
    reference = after["fingerprints"]
    guards, binaries, inputs = {}, {}, {}
    suffix = ".exe" if os.name == "nt" else ""
    for label, path, data in (("before", before_path, before), ("after", after_path, after)):
        provenance = data["provenance"]
        files = {
            path: sha(path),
            path.parent / provenance["executable"]: provenance["bcore_executable_sha256"],
            path.parent / "source.zip": provenance["source_archive_sha256"],
            path.parent / "source-manifest.json": provenance["source_manifest_sha256"],
        }
        verify_files(path.parent, files)
        binary = output / (label + suffix)
        freeze_copy(path.parent / provenance["executable"], binary, provenance["bcore_executable_sha256"])
        guards.update(files)
        guards[binary] = provenance["bcore_executable_sha256"]
        binaries[label] = binary
        inputs[label] = {"results_path": str(path), "results_sha256": files[path],
                         "bcore_executable_sha256": provenance["bcore_executable_sha256"],
                         "source_archive_sha256": provenance["source_archive_sha256"]}
    result = {"schema": 1, "scope": after["scope"], "recorded_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
              "method": "BCore-only fresh processes, alternating before/after order for each pair; same recorded workload",
              "inputs": inputs, "environment": environment, "contract": after["contract"],
              "workers": after["workers"], "seed": after["seed"], "fingerprints": reference,
              "samples": []}
    write_json(output / "intent.json", result)
    for workers in after["workers"]:
        for process in range(after["processes_per_configuration"]):
            for label in (("before", "after") if process % 2 == 0 else ("after", "before")):
                name = f"{label}-{workers}w-{process + 1}"
                request = {"schema": after["schema"], "scope": after["scope"], "seed": after["seed"],
                           "workers": workers, "warmup_batches": after["warmup_batches_per_process"],
                           "measured_batches": after["measured_batches_per_process"], "chunks": CHUNKS}
                request_path = output / (name + ".request.json")
                write_json(request_path, request)
                log = output / (name + ".log")
                command = [str(binaries[label]), str(request_path)]
                verify_files(output, guards)
                print(f"Running {name}", flush=True)
                execute(command, output, log, 900, env)
                verify_files(output, guards)
                rows = [line.split("NOISE_BENCHMARK=", 1)[1] for line in log.read_text(encoding="utf-8").splitlines()
                        if "NOISE_BENCHMARK=" in line]
                if len(rows) != 1:
                    raise ValueError(f"expected one result: {log}")
                sample = json.loads(rows[0])
                validate_sample(sample, "BCore", request)
                if sample["fingerprints"] != reference:
                    raise ValueError(f"block/mark/WG mismatch: {name}")
                result["samples"].append({**sample, "variant": label, "process": process + 1,
                    "command": command, "request_sha256": sha(request_path), "log_sha256": sha(log),
                    "artifacts_verified_before_after": True})
                write_json(output / "progress.json", result, exclusive=False)
                time.sleep(1)
    result["summary"] = []
    for workers in after["workers"]:
        values = {}
        for label in ("before", "after"):
            medians = [statistics.median(s["seconds"]) for s in result["samples"]
                       if s["variant"] == label and s["workers"] == workers]
            values[label] = {"process_median_seconds": medians,
                             "chunks_per_second": len(CHUNKS) / statistics.median(medians)}
        result["summary"].append({"workers": workers, **values,
            "speedup": values["after"]["chunks_per_second"] / values["before"]["chunks_per_second"]})
    verify_files(output, guards)
    result["verified_matching_outputs"] = True
    write_json(output / "results.json", result)
    print(json.dumps(result["summary"], indent=2), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    target = (ROOT / "target").resolve()
    if not output.is_relative_to(target) or output == target or output.exists():
        parser.error("output must be a new directory within target/")
    output.mkdir(parents=True)
    try:
        compare(args.before.resolve(), args.after.resolve(), output)
    except Exception as error:
        write_json(output / "failure.json", {"error_type": type(error).__name__, "error": str(error)})
        raise


if __name__ == "__main__":
    main()
