"""Build and run the shared-context BCore diagnostic with frozen source provenance."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import zipfile

from oracle import compare, replay_plan, sha

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


def sources():
    files = []
    for directory in (ROOT / "crates/bcore-core", ROOT / "crates/bcore-worldgen", HERE):
        files.extend(p for p in directory.rglob("*") if p.is_file() and p.suffix in (".rs", ".toml", ".json", ".lock"))
    return {p.relative_to(ROOT).as_posix(): p.read_bytes() for p in sorted(set(files))}


def hashes(files):
    return {path: hashlib.sha256(data).hexdigest() for path, data in files.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=1200)
    parser.add_argument("--reuse-build", type=Path, help="reuse a prior frozen diagnostic binary and its exact source provenance")
    parser.add_argument("--feature-case", type=Path, help="run the isolated production forest-grass kernel on exported native inputs")
    parser.add_argument("--trace-feature", help="observe one production source feature as x,z,feature")
    args = parser.parse_args()
    if args.feature_case and args.trace_feature:
        parser.error("feature-case and trace-feature are separate diagnostics")
    if "CARGO_TARGET_DIR" not in os.environ:
        parser.error("use the coordinator-supplied CARGO_TARGET_DIR")
    out = args.output.resolve()
    if not out.is_relative_to(ROOT / "target") or out.exists():
        parser.error("output must be a new isolated directory under target/")
    out.mkdir(parents=True)
    config = out / "replay.json"
    program = "native-feature-replay" if args.feature_case else "native-history-replay"
    if args.feature_case:
        config.write_bytes(args.feature_case.read_bytes())
        if json.loads(config.read_text())["native_provenance"] != sha(args.capture / "provenance.json"):
            raise RuntimeError("feature input/native capture provenance mismatch")
    else:
        plan = replay_plan(args.capture)
        if args.trace_feature:
            x, z, feature = args.trace_feature.split(",", 2)
            plan["feature_trace"] = {"source": [int(x), int(z)], "feature": feature}
        config.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    exe = out / (program + ".exe")
    if args.reuse_build:
        prior = json.loads((args.reuse_build / "provenance.json").read_text())
        if args.trace_feature and prior.get("feature_trace_version") != 1:
            raise RuntimeError("the archived binary does not support feature-boundary diagnostics")
        previous_exe = args.reuse_build / (program + ".exe")
        if sha(previous_exe) != prior["binary_sha256"]:
            raise RuntimeError("frozen replay binary digest mismatch")
        shutil.copy2(previous_exe, exe)
        shutil.copy2(args.reuse_build / "source.zip", out / "source.zip")
        provenance = {"reused_build": str(args.reuse_build.resolve()), "reused_provenance_sha256": sha(args.reuse_build / "provenance.json"),
                      "source_sha256": prior["source_sha256"], "binary_sha256": prior["binary_sha256"],
                      "feature_trace_version": prior.get("feature_trace_version")}
    else:
        before = sources()
        if hashes(before) != hashes(sources()):
            raise RuntimeError("production files changed during source snapshot; preserve this attempt and retry in a new output")
        source_root = out / "source"
        with zipfile.ZipFile(out / "source.zip", "w", zipfile.ZIP_DEFLATED) as archive:
            for path, data in before.items():
                archive.writestr(path, data)
                copy = source_root / path
                copy.parent.mkdir(parents=True, exist_ok=True)
                copy.write_bytes(data)
        build_command = ["cargo", "build", "--manifest-path", str(source_root / "scripts/native-generation-reference/Cargo.toml"),
                         "--release", "-j", "1", "--locked", "--bin", program]
        with (out / "build.log").open("w", encoding="utf-8") as log:
            build = subprocess.run(build_command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
        provenance = {"build_command": build_command, "target_dir": os.environ["CARGO_TARGET_DIR"],
                      "feature_trace_version": 1,
                      "source_sha256": hashes(before), "source_snapshot": str(source_root),
                      "build_returncode": build.returncode,
                      "rustc": subprocess.run(["rustc", "--version"], capture_output=True, text=True, check=True).stdout.strip()}
        if build.returncode:
            (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
            raise RuntimeError(f"BCore compile failed; preserved {out / 'build.log'}")
        shutil.copy2(Path(os.environ["CARGO_TARGET_DIR"]) / "release" / (program + ".exe"), exe)
    provenance.update({"program": program, "runner_sha256": sha(Path(__file__)), "source_zip_sha256": sha(out / "source.zip"),
                       "replay_sha256": sha(config), "native_provenance_sha256": sha(args.capture / "provenance.json")})
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    result_path = out / ("feature-comparison.json" if args.feature_case else "chunks.jsonl")
    command = [str(exe.resolve()), str(config), str(result_path)]
    env = {**os.environ, "RAYON_NUM_THREADS": "1"}
    with (out / "run.log").open("w", encoding="utf-8") as log:
        subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=args.timeout)
    provenance.update({"run_command": command, "binary_sha256": sha(exe), "rayon_num_threads": 1,
                       "result_sha256": sha(result_path)})
    if args.trace_feature:
        provenance["feature_trace_sha256"] = sha(out / "feature_traces.jsonl")
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    if args.feature_case:
        print(json.dumps({"output": str(out), **json.loads(result_path.read_text())}, indent=2))
        return
    diff = compare(args.capture, result_path)
    (out / "comparison.json").write_text(json.dumps(diff, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(out), "requests": [
        {"spec": r["spec"], "states": r["states"]["mismatches"], "first": r["states"]["first"],
         "biomes": r["biomes"]["mismatches"], "source_order": r["source_order"],
         "serialized_light": {k: v for k, v in r["serialized_light"].items() if k != "layers"},
         "generated_metadata": r["generated_metadata"]}
        for r in diff["requests"]]}, indent=2))


if __name__ == "__main__":
    main()
