"""Measure fresh NOISE fill through WG heightmaps (noise-fill-wg-v2).

Build from an archived source snapshot and launch a frozen per-run executable.
Warmup timings are retained but excluded from summaries. This revised boundary
must never be spliced into the historical October 5 noise-fill-kernel series.
Run exclusively through the project's heavy-work coordinator when one is active.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import time

from provenance import (JAVA_SOURCES, cargo_config_hashes, controlled_environment, freeze_copy, sha,
                        snapshot, source_hashes, verify_files, write_json)

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = 2
SCOPE = "noise-fill-wg-v2"
JAR_SHA = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
CHUNKS = [[0, 0], [1, 0], [0, 1], [1, 1], [-64, -32], [-63, -32], [62, 0], [-125, 187]]
HASH_FIELDS = ("blocks_sha256", "marks_sha256", "world_surface_wg_sha256", "ocean_floor_wg_sha256")
CONTRACT = {
    "schema": SCHEMA, "scope": SCOPE, "chunks": CHUNKS, "min_y": -64, "height": 384,
    "worker_budgets": [1, 2, 4], "initial_state": "fresh empty chunks, empty structure references, empty Blender",
    "timed": ["entry-point dispatch and completion", "fresh per-chunk density/cache/aquifer setup",
              "complete material fill including noise ore veins", "fluid postprocessing requests",
              "both WORLD_SURFACE_WG and OCEAN_FLOOR_WG heightmaps"],
    "implementation_costs": {
        "BCore": ["production surface-biome bookkeeping (256 column lookups/chunk)",
                  "column scratch, per-column material resources, density cache cleanup, mark ordering",
                  "normal post-NOISE capture_worldgen_heightmaps(false)"],
        "Vanilla 26.1": ["reflection lookup/invocation per chunk, async submission/join",
                         "NoiseChunk/aquifer setup, scratch/palette growth, section acquire/release",
                         "in-task WG heightmap creation/updates"],
    },
    "excluded": ["build and process/world startup", "initial chunk allocation", "fingerprinting/JSON",
                 "BIOMES stage (Rust surface-biome bookkeeping remains timed)", "structure generation",
                 "SURFACE/CARVERS/FEATURES/LIGHT/SPAWN/FULL", "packets/disk I/O/gameplay"],
    "warmup": "positive fixed count; all durations retained; lazy Rust seeded resources warm on workers; convergence requires review",
    "fingerprints": "SHA-256: block IDs u32 LE Y/Z/X; marks i32 LE X/Y/Z grouped by ascending section, insertion order/duplicates retained; each WG map 256 first-free i32 LE Y heights in Z/X order",
    "worker_contract": "Rayon N; actual native ForkJoinPool parallelism N, ActiveProcessorCount=N+1; not an OS CPU-affinity limit",
    "method": "serial fresh processes in alternating engine order; median of process-median measured batch times; warmups excluded",
    "historical_comparability": "new boundary, not an optimization of schema 1 noise-fill-kernel",
}


def command_version(command, cwd, env):
    result = subprocess.run(command, cwd=cwd, env=env, check=True, capture_output=True, text=True)
    return (result.stdout + result.stderr).strip()


def execute(command, cwd, log, timeout, env):
    with log.open("x", encoding="utf-8") as stream:
        subprocess.run(command, cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT,
                       timeout=timeout, check=True)


def validate_sample(sample, engine, request):
    expected = {"schema": SCHEMA, "scope": SCOPE, "engine": engine, "seed": request["seed"],
                "workers": request["workers"], "warmup_batches": request["warmup_batches"],
                "chunks_per_batch": len(CHUNKS)}
    if any(sample.get(k) != v for k, v in expected.items()):
        raise ValueError("invalid benchmark identity/contract")
    for key, count in (("seconds", request["measured_batches"]), ("warmup_seconds", request["warmup_batches"])):
        values = sample.get(key)
        if not isinstance(values, list) or len(values) != count or any(
                type(v) not in (int, float) or not math.isfinite(v) or v <= 0 for v in values):
            raise ValueError(f"invalid benchmark {key}")
    if engine == "Vanilla 26.1" and (sample.get("native_executor") != "ForkJoinPool"
            or sample.get("available_processors") != request["workers"] + 1):
        raise ValueError("native executor verification missing or inconsistent")
    fingerprints = sample.get("fingerprints")
    if not isinstance(fingerprints, list) or len(fingerprints) != len(CHUNKS):
        raise ValueError("missing chunk fingerprints")
    for pos, item in zip(CHUNKS, fingerprints):
        if item.get("chunk") != pos or type(item.get("marks")) is not int or item["marks"] < 0:
            raise ValueError("invalid chunk/mark fingerprint identity")
        if any(not isinstance(item.get(k), str) or not re.fullmatch("[0-9a-f]{64}", item[k]) for k in HASH_FIELDS):
            raise ValueError("missing or invalid block/mark/WG digest")


def summaries(samples, workers):
    result = []
    for count in workers:
        for engine in ("Vanilla 26.1", "BCore"):
            medians = [statistics.median(s["seconds"]) for s in samples
                       if s["engine"] == engine and s["workers"] == count]
            median = statistics.median(medians)
            result.append({"engine": engine, "workers": count, "median_batch_seconds": median,
                           "min_process_median_seconds": min(medians), "max_process_median_seconds": max(medians),
                           "chunks_per_second": len(CHUNKS) / median})
    return result


def compare_contract(baseline, current):
    for field in ("schema", "scope", "contract", "seed", "chunks", "workers", "hardware",
                  "processes_per_configuration", "warmup_batches_per_process", "measured_batches_per_process"):
        if baseline.get(field) != current[field]:
            raise ValueError(f"comparison contract differs: {field}")
    for field in ("harness_source_sha256", "native_jar_sha256", "native_libraries_sha256", "native_classes_sha256", "runtime_sha256",
                  "rustc", "cargo", "java", "javac", "environment", "java_flags", "cargo_config_sha256"):
        if baseline.get("provenance", {}).get(field) != current["provenance"][field]:
            raise ValueError(f"comparison provenance differs: {field}")
    if baseline.get("verified_matching_outputs") is not True:
        raise ValueError("baseline is not a completed matched-output run")


def run(args, output):
    env, environment = controlled_environment()
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    if sha(jar) != JAR_SHA:
        raise ValueError("native JAR digest differs from pinned 26.1")
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    if not libraries:
        raise ValueError("missing native libraries")
    initial = source_hashes(ROOT)
    source, provenance = snapshot(ROOT, output, initial)
    native_build = output / "native-build"
    native_build.mkdir()
    binary_dir = output / "bin"
    binary_dir.mkdir()
    target_dir = args.target_dir.resolve()
    # The build cache may be reused serially, but only the verified private copy
    # below is ever measured. Cargo builds the archived tree, not mutable ROOT.
    build_command = ["cargo", "build", "--manifest-path", str(source / "Cargo.toml"), "-p", "bcore-worldgen",
                     "--release", "--locked", "--verbose", "-j", "1", "--features", "diagnostics",
                     "--example", "noise_bench", "--target-dir", str(target_dir)]
    java = str(args.java.resolve())
    javac_command = ["javac", "-d", str(native_build), *[str(source / p) for p in JAVA_SOURCES]]
    provenance.update({"git_head": command_version(["git", "--no-optional-locks", "rev-parse", "HEAD"], ROOT, env),
                       "rustc": command_version(["rustc", "-Vv"], source, env),
                       "cargo": command_version(["cargo", "--version"], source, env),
                       "java": command_version([java, "-version"], source, env),
                       "javac": command_version(["javac", "-version"], source, env),
                       "environment": environment, "build_command": build_command, "javac_command": javac_command,
                       "cargo_config_sha256": cargo_config_hashes(source, env),
                       "java_flags": ["-Xmx2G", "-XX:ActiveProcessorCount=N+1", "-Dmax.bg.threads=N"],
                       "native_jar_sha256": JAR_SHA,
                       "native_libraries_sha256": {p.relative_to(ROOT / "target/vanilla-775/libraries").as_posix(): sha(p) for p in libraries}})
    runtime_files = [Path(java)]
    for name in ("release", "lib/modules"):
        path = Path(java).parent.parent / name
        if path.is_file():
            runtime_files.append(path)
    provenance["runtime_sha256"] = {p.relative_to(Path(java).parent.parent).as_posix(): sha(p) for p in runtime_files}
    # Write intent before compiling so failed attempts keep their provenance.
    write_json(output / "build-plan.json", provenance)
    execute(javac_command, source, output / "javac.log", 180, env)
    execute(build_command, source, output / "cargo.log", 3600, env)
    suffix = ".exe" if os.name == "nt" else ""
    executable = binary_dir / ("noise_bench" + suffix)
    provenance["bcore_executable_sha256"] = freeze_copy(target_dir / "release/examples" / executable.name, executable)
    provenance["executable"] = executable.relative_to(output).as_posix()
    provenance["native_classes_sha256"] = {p.relative_to(native_build).as_posix(): sha(p) for p in sorted(native_build.rglob("*.class"))}
    if not provenance["native_classes_sha256"]:
        raise ValueError("native compilation produced no classes")
    provenance["build_logs_sha256"] = {name: sha(output / name) for name in ("cargo.log", "javac.log")}
    verify_files(source, initial)
    if source_hashes(ROOT) != initial:
        raise ValueError("working sources changed during build")
    write_json(output / "provenance.json", provenance)
    hardware = {"os": platform.platform(), "logical_processors": os.cpu_count()}
    if os.name == "nt":
        query = "Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors | ConvertTo-Json -Compress"
        hardware["cpu"] = json.loads(command_version(["powershell", "-NoProfile", "-Command", query], source, env))
    result = {"schema": SCHEMA, "scope": SCOPE, "label": args.label,
              "recorded_utc": dt.datetime.now(dt.timezone.utc).isoformat(), "contract": CONTRACT,
              "minecraft": "26.1", "protocol": 775, "seed": str(args.seed), "chunks": CHUNKS,
              "min_y": -64, "height": 384, "workers": args.workers, "hardware": hardware,
              "processes_per_configuration": args.processes, "warmup_batches_per_process": args.warmup,
              "measured_batches_per_process": args.batches, "provenance": provenance, "samples": []}
    baseline = None
    if args.compare_to:
        baseline = json.loads(args.compare_to.read_text(encoding="utf-8"))
        compare_contract(baseline, result)
        result["comparison_baseline"] = {"path": str(args.compare_to.resolve()), "sha256": sha(args.compare_to)}
    classpath = os.pathsep.join(map(str, [native_build, jar, *libraries]))
    guards = {str(executable): provenance["bcore_executable_sha256"], str(jar): JAR_SHA,
              str(output / "source.zip"): provenance["source_archive_sha256"],
              str(output / "source-manifest.json"): provenance["source_manifest_sha256"]}
    guards.update({str(p): provenance["native_libraries_sha256"][p.relative_to(ROOT / "target/vanilla-775/libraries").as_posix()] for p in libraries})
    guards.update({str(p): provenance["runtime_sha256"][p.relative_to(Path(java).parent.parent).as_posix()] for p in runtime_files})
    guards.update({str(native_build / name): digest for name, digest in provenance["native_classes_sha256"].items()})
    guards[str(output / "provenance.json")] = sha(output / "provenance.json")
    guards.update(provenance["cargo_config_sha256"])
    if args.compare_to:
        guards[str(args.compare_to.resolve())] = result["comparison_baseline"]["sha256"]
    reference = baseline["fingerprints"] if baseline else None
    for workers in args.workers:
        for process in range(args.processes):
            for engine in (["Vanilla 26.1", "BCore"] if process % 2 == 0 else ["BCore", "Vanilla 26.1"]):
                name = f"{engine.split()[0].lower()}-{workers}w-{process + 1}"
                request = {"schema": SCHEMA, "scope": SCOPE, "seed": str(args.seed), "workers": workers,
                           "warmup_batches": args.warmup, "measured_batches": args.batches, "chunks": CHUNKS}
                request_path = output / f"{name}.request.json"
                write_json(request_path, request)
                command = ([str(executable), str(request_path)] if engine == "BCore" else
                           [java, "-Xmx2G", f"-XX:ActiveProcessorCount={workers + 1}",
                            f"-Dmax.bg.threads={workers}", "-cp", classpath, "NoiseFillBenchmark", str(request_path)])
                verify_files(output, guards)
                print(f"Running {name}", flush=True)
                log = output / f"{name}.log"
                execute(command, output, log, 900, env)
                verify_files(output, guards)
                rows = [line.split("NOISE_BENCHMARK=", 1)[1] for line in log.read_text(encoding="utf-8").splitlines()
                        if "NOISE_BENCHMARK=" in line]
                if len(rows) != 1:
                    raise ValueError(f"expected one benchmark result: {log}")
                sample = json.loads(rows[0])
                validate_sample(sample, engine, request)
                if reference is None:
                    reference = sample["fingerprints"]
                if reference != sample["fingerprints"]:
                    write_json(output / f"{name}.mismatch.json", {"reference": reference, "actual": sample})
                    raise ValueError(f"block/mark/WG hashes differ: {name}")
                result["samples"].append({**sample, "process": process + 1, "command": command,
                    "request_sha256": sha(request_path), "log_sha256": sha(log), "artifacts_verified_before_after": True,
                    "launched_artifact_sha256": provenance["bcore_executable_sha256"] if engine == "BCore" else JAR_SHA})
                write_json(output / "progress.json", result, exclusive=False)
                time.sleep(1)
    verify_files(output, guards)
    verify_files(source, initial)
    if source_hashes(ROOT) != initial:
        raise ValueError("working sources changed while measuring; retain this failed attempt")
    result.update(summary=summaries(result["samples"], args.workers), fingerprints=reference, verified_matching_outputs=True)
    write_json(output / "results.json", result)
    print(json.dumps({"output": str(output), "scope": SCOPE, "summary": result["summary"],
                      "verified_matching_outputs": True, "results_sha256": sha(output / "results.json")}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, default=Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target/build")))
    parser.add_argument("--java", type=Path, default=ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe")
    parser.add_argument("--seed", type=int, default=846692123413862008)
    parser.add_argument("--workers", nargs="+", type=int, choices=(1, 2, 4), default=[1, 2, 4])
    parser.add_argument("--processes", type=int, default=3)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--batches", type=int, default=5)
    parser.add_argument("--label", default="unlabelled")
    parser.add_argument("--compare-to", type=Path, help="require identical v2 harness/conditions and baseline fingerprints")
    args = parser.parse_args()
    output = args.output.resolve()
    target_root = (ROOT / "target").resolve()
    if not output.is_relative_to(target_root) or output == target_root or output.exists():
        parser.error("output must be a new directory inside this workspace target/")
    if not args.target_dir.resolve().is_relative_to(target_root):
        parser.error("build target directory must be inside this workspace target/")
    if args.processes < 1 or args.batches < 1 or args.warmup < 1 or len(set(args.workers)) != len(args.workers):
        parser.error("positive sample/warmup counts and unique workers are required")
    if not -(1 << 63) <= args.seed < (1 << 63):
        parser.error("seed must fit a signed 64-bit integer")
    output.mkdir(parents=True)
    try:
        run(args, output)
    except Exception as error:
        write_json(output / "failure.json", {"schema": SCHEMA, "scope": SCOPE,
                   "error_type": type(error).__name__, "error": str(error)})
        raise


if __name__ == "__main__":
    main()
