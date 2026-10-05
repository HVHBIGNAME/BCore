"""Measure original Vanilla / BCore NOISE fills and reject differing block/effect hashes.

Fresh processes run serially in alternating engine order. Startup, warmups, chunk
allocation, hashing and output are untimed. This is a material-fill benchmark,
not a full-server/TPS or complete world-generation benchmark.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
JAR_SHA = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
SOURCES = [ROOT / p for p in (
    "scripts/TreeReference.java", "scripts/NativeWorldgenRegistries.java",
    "scripts/jigsaw-reference/JigsawSupport.java",
    "scripts/density-materials-reference/DensityMaterialsReference.java",
    "scripts/benchmarks/NoiseFillBenchmark.java",
)]
CHUNKS = [[0, 0], [1, 0], [0, 1], [1, 1], [-64, -32], [-63, -32], [62, 0], [-125, 187]]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hashes():
    files = set(SOURCES + [Path(__file__)])
    files.update((ROOT / "crates/bcore-worldgen/src").rglob("*.rs"))
    files.add(ROOT / "crates/bcore-worldgen/examples/noise_bench.rs")
    return {p.relative_to(ROOT).as_posix(): sha(p) for p in sorted(files)}


def command_version(command):
    result = subprocess.run(command, check=True, capture_output=True, text=True)
    return (result.stdout + result.stderr).strip()


def execute(command, cwd, log, timeout):
    with log.open("x", encoding="utf-8") as stream:
        subprocess.run(command, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT,
                       timeout=timeout, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/full-parity-20261002/build-native-history")
    parser.add_argument("--java", type=Path, default=ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe")
    parser.add_argument("--seed", type=int, default=846692123413862008)
    parser.add_argument("--workers", nargs="+", type=int, default=[1, 2, 4])
    parser.add_argument("--processes", type=int, default=3)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--batches", type=int, default=5)
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / "target") or output.exists():
        parser.error("output must be a new, never-reused directory inside target/")
    if args.processes < 1 or args.batches < 1 or args.warmup < 0 or any(n < 1 or n > 64 for n in args.workers):
        parser.error("invalid sample/worker count")
    if not args.target_dir.resolve().is_relative_to(ROOT / "target"):
        parser.error("target directory must be inside the workspace target/")
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    if sha(jar) != JAR_SHA:
        parser.error("native JAR digest differs from pinned 26.1")
    output.mkdir(parents=True)
    build = output / "native-build"
    build.mkdir()
    initial_hashes = source_hashes()
    execute(["javac", "-d", str(build), *map(str, SOURCES)], ROOT, output / "javac.log", 180)
    for source in SOURCES:
        (build / source.name).write_bytes(source.read_bytes())
    suffix = ".exe" if os.name == "nt" else ""
    executable = args.target_dir.resolve() / "release/examples" / ("noise_bench" + suffix)
    if not args.skip_build:
        execute(["cargo", "build", "-p", "bcore-worldgen", "--release", "--locked", "-j", "1",
                 "--features", "diagnostics", "--example", "noise_bench", "--target-dir", str(args.target_dir.resolve())],
                ROOT, output / "cargo.log", 3600)
    classpath = os.pathsep.join(map(str, [build, jar, *sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))]))
    hardware = {"os": platform.platform(), "logical_processors": os.cpu_count()}
    if os.name == "nt":
        query = "Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors | ConvertTo-Json -Compress"
        hardware["cpu"] = json.loads(subprocess.run(["powershell", "-NoProfile", "-Command", query],
                                                    check=True, capture_output=True, text=True).stdout)
    result = {
        "schema": 1, "recorded_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "scope": "noise-fill-kernel", "minecraft": "26.1", "protocol": 775,
        "seed": str(args.seed), "chunks": CHUNKS, "min_y": -64, "height": 384,
        "hardware": hardware, "processes_per_configuration": args.processes,
        "warmup_batches_per_process": args.warmup, "measured_batches_per_process": args.batches,
        "timed": "Production NOISE material fill of eight independent fresh chunks, including aquifers, ore veins and fluid postprocessing requests. Empty structure references; no blending.",
        "excluded": ["compilation", "registry/JVM startup", "warmup batches", "initial chunk allocation",
                     "output checksums", "BIOMES", "structure admission/placement", "SURFACE", "CARVERS",
                     "FEATURES", "LIGHT", "SPAWN", "FULL", "packets", "disk I/O", "gameplay/TPS"],
        "worker_contract": "BCore Rayon pool N; Vanilla max.bg.threads=N and ActiveProcessorCount=N+1. Worker budget is not an OS CPU-affinity cap.",
        "method": "Serial fresh processes, alternating engine order; medians of process-median batch times. Every batch and every engine/worker run must produce the same complete block and ordered per-section postprocessing hashes.",
        "provenance": {"native_jar_sha256": JAR_SHA, "bcore_executable_sha256": sha(executable),
                       "source_sha256": initial_hashes, "rustc": command_version(["rustc", "--version"]),
                       "java": command_version([str(args.java.resolve()), "-version"])},
        "samples": [], "summary": [],
    }
    reference = None
    for workers in args.workers:
        for process in range(args.processes):
            for engine in (["Vanilla 26.1", "BCore"] if process % 2 == 0 else ["BCore", "Vanilla 26.1"]):
                name = f"{engine.split()[0].lower()}-{workers}w-{process + 1}"
                request = {"seed": str(args.seed), "workers": workers, "warmup_batches": args.warmup,
                           "measured_batches": args.batches, "chunks": CHUNKS}
                request_path = output / f"{name}.request.json"
                request_path.write_text(json.dumps(request, indent=2) + "\n", encoding="utf-8")
                command = ([str(executable), str(request_path)] if engine == "BCore" else
                           [str(args.java.resolve()), "-Xmx2G", f"-XX:ActiveProcessorCount={workers + 1}",
                            f"-Dmax.bg.threads={workers}", "-cp", classpath, "NoiseFillBenchmark", str(request_path)])
                print(f"Running {name}", flush=True)
                log = output / f"{name}.log"
                execute(command, build if engine != "BCore" else ROOT, log, 900)
                rows = [line.split("NOISE_BENCHMARK=", 1)[1] for line in log.read_text(encoding="utf-8").splitlines()
                        if "NOISE_BENCHMARK=" in line]
                if len(rows) != 1:
                    raise ValueError(f"expected one benchmark result: {log}")
                sample = json.loads(rows[0])
                if sample["engine"] != engine or sample["workers"] != workers or len(sample["seconds"]) != args.batches:
                    raise ValueError(f"invalid benchmark identity/sample count: {name}")
                if reference is None:
                    reference = sample["fingerprints"]
                if reference != sample["fingerprints"]:
                    (output / f"{name}.mismatch.json").write_text(json.dumps({"reference": reference, "actual": sample}, indent=2))
                    raise ValueError(f"block/effect hashes differ; do not publish a matched-output performance comparison: {name}")
                result["samples"].append({**sample, "process": process + 1, "log_sha256": sha(log)})
                (output / "progress.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
                time.sleep(1)
        for engine in ["Vanilla 26.1", "BCore"]:
            samples = [s for s in result["samples"] if s["engine"] == engine and s["workers"] == workers]
            medians = [statistics.median(s["seconds"]) for s in samples]
            median = statistics.median(medians)
            result["summary"].append({"engine": engine, "workers": workers,
                                      "median_batch_seconds": median, "min_process_median_seconds": min(medians),
                                      "max_process_median_seconds": max(medians),
                                      "chunks_per_second": len(CHUNKS) / median})
    if source_hashes() != initial_hashes:
        raise ValueError("benchmark sources changed while measuring; preserve this run but do not publish it")
    result["verified_matching_outputs"] = True
    result["fingerprints"] = reference
    with (output / "results.json").open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"output": str(output), "summary": result["summary"],
                      "verified_matching_outputs": True}, indent=2))


if __name__ == "__main__":
    main()
