"""Build and run the shared-context BCore diagnostic with frozen source provenance."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import shutil
import subprocess
import tomllib
import zipfile

from oracle import (
    NATIVE_INPUT_FILES, OBSERVATION_CONTRACT, REPLAY_SCHEMA, compare,
    native_input_identity, replay_plan, same_value, sha, validate_replay_plan,
)

ROOT = Path(__file__).resolve().parents[2]
HISTORY_PROGRAM = "native-history-replay"
HISTORY_CONTRACT = {"program": HISTORY_PROGRAM, "schema": REPLAY_SCHEMA,
                    "observation_contract": OBSERVATION_CONTRACT, "feature_trace_version": 1}
OBSERVER = "scripts/native-generation-reference/"
REQUIRED_TOOLS = {OBSERVER + name for name in ("replay_bcore.py", "oracle.py", "test_oracle.py", "README.md")}


def sources(root=ROOT):
    files = []
    for directory in (root / "crates/bcore-core", root / "crates/bcore-registry", root / "crates/bcore-worldgen"):
        files.extend(p for p in directory.rglob("*") if p.is_file() and p.suffix in (".rs", ".toml", ".json", ".lock"))
    files.extend(p for p in (root / OBSERVER).iterdir()
                 if p.is_file() and p.suffix in (".rs", ".toml", ".json", ".lock", ".py", ".java", ".md"))
    files.append(root / "rust-toolchain.toml")
    return {p.relative_to(root).as_posix(): p.read_bytes() for p in sorted(set(files))}


def tool_sources(root=ROOT):
    files = [p for p in (root / OBSERVER).iterdir() if p.is_file() and p.suffix in (".py", ".md")]
    files.append(root / "scripts/run_generation_check.py")
    return {p.relative_to(root).as_posix(): p.read_bytes() for p in sorted(files)}


def hashes(files):
    return {path: hashlib.sha256(data).hexdigest() for path, data in files.items()}


def write_archive(path, files, source_root=None):
    with zipfile.ZipFile(path, "x", zipfile.ZIP_DEFLATED) as archive:
        for name, data in sorted(files.items()):
            archive.writestr(name, data)
            if source_root is not None:
                copy = source_root / name
                copy.parent.mkdir(parents=True, exist_ok=True)
                copy.write_bytes(data)


def validate_archive(path, digest, expected, required=()):
    if sha(path) != digest:
        raise ValueError(f"frozen {path.name} digest mismatch")
    if not isinstance(expected, dict) or not expected or not set(required) <= expected.keys():
        raise ValueError(f"{path.name} lacks required source/tool provenance")
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(set(name.casefold() for name in names)) != len(names):
            raise ValueError(f"duplicate source archive member in {path.name}")
        for name in names:
            entry = PurePosixPath(name)
            if (entry.is_absolute() or entry.as_posix() != name or ".." in entry.parts
                    or "\\" in name or ":" in name):
                raise ValueError(f"noncanonical source archive member: {name}")
        if set(names) != set(expected):
            raise ValueError(f"{path.name} source manifest membership mismatch")
        for name in names:
            if hashlib.sha256(archive.read(name)).hexdigest() != expected[name]:
                raise ValueError(f"source member digest mismatch: {name}")


def validate_source_dependencies(path):
    with zipfile.ZipFile(path) as archive:
        members = set(archive.namelist())
        for name in sorted(members):
            if PurePosixPath(name).name != "Cargo.toml":
                continue
            manifest = tomllib.loads(archive.read(name).decode("utf-8"))
            tables = [manifest, *manifest.get("target", {}).values()]
            for table in tables:
                for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
                    for dependency in table.get(kind, {}).values():
                        if not isinstance(dependency, dict) or "path" not in dependency:
                            continue
                        relative = dependency["path"]
                        if PurePosixPath(relative).is_absolute() or ":" in relative or "\\" in relative:
                            raise ValueError(f"source archive has an external path dependency: {name}")
                        required = posixpath.normpath(str(PurePosixPath(name).parent / relative / "Cargo.toml"))
                        if required.startswith("../") or required not in members:
                            raise ValueError(f"source archive lacks path dependency {required}")


def validate_reuse_build(directory, program, trace_feature=False):
    prior = json.loads((directory / "provenance.json").read_text(encoding="utf-8"))
    if prior.get("program") != program:
        raise ValueError("reused build program mismatch")
    if program == HISTORY_PROGRAM:
        for field, expected in (("snapshot_boundary_version", REPLAY_SCHEMA), ("replay_schema", REPLAY_SCHEMA),
                                ("observation_contract", OBSERVATION_CONTRACT), ("binary_contract", HISTORY_CONTRACT)):
            if not same_value(prior.get(field), expected):
                raise ValueError(f"reused history build {field} is not schema 3; create a fresh build")
    if trace_feature and not same_value(prior.get("feature_trace_version"), 1):
        raise ValueError("archived binary does not support feature-boundary diagnostics")
    if not same_value(prior.get("build_returncode"), 0):
        raise ValueError("reused build lacks a successful compile receipt")
    if (not isinstance(prior.get("build_command"), list) or not prior["build_command"]
            or not isinstance(prior.get("rustc"), str) or not prior["rustc"]
            or type(prior.get("feature_trace_version")) is not int):
        raise ValueError("reused build lacks command/compiler/version provenance")
    if sha(directory / (program + ".exe")) != prior.get("binary_sha256"):
        raise ValueError("frozen replay binary digest mismatch")
    required = REQUIRED_TOOLS | {OBSERVER + "Cargo.toml", OBSERVER + "Cargo.lock",
        OBSERVER + ("replay.rs" if program == HISTORY_PROGRAM else "feature_replay.rs"),
        "crates/bcore-core/Cargo.toml", "crates/bcore-worldgen/Cargo.toml", "rust-toolchain.toml"}
    validate_archive(directory / "source.zip", prior.get("source_zip_sha256"), prior.get("source_sha256"), required)
    validate_source_dependencies(directory / "source.zip")
    validate_archive(directory / "tools.zip", prior.get("tools_zip_sha256"), prior.get("tool_sha256"), REQUIRED_TOOLS)
    for field, name in (("runner_sha256", "replay_bcore.py"), ("comparator_sha256", "oracle.py")):
        if prior.get(field) != prior["tool_sha256"][OBSERVER + name]:
            raise ValueError(f"reused {field} tool provenance mismatch")
    if sha(directory / "replay.json") != prior.get("replay_sha256"):
        raise ValueError("reused replay plan digest mismatch")
    if program == HISTORY_PROGRAM:
        plan = json.loads((directory / "replay.json").read_text(encoding="utf-8"))
        validate_replay_plan(plan)
        for field in NATIVE_INPUT_FILES:
            if prior.get(field) != plan[field]:
                raise ValueError(f"reused plan/provenance {field} mismatch")
    return prior


def validate_binary_contract(exe):
    result = subprocess.run([str(exe.resolve()), "--version"], capture_output=True, text=True, check=True, timeout=30)
    contract = json.loads(result.stdout)
    if not same_value(contract, HISTORY_CONTRACT):
        raise ValueError("replay executable does not advertise the schema 3 returned-object contract")
    return contract


def comparison_summary(diff):
    def snapshot(row):
        return {"pos": row["pos"], "scored_matches": row.get("requested_scored_matches", row["scored_matches"]), "presence": row["presence"],
                "states": row.get("states", {}).get("mismatches"),
                "first_state": row.get("states", {}).get("first"),
                "biomes": row.get("biomes", {}).get("mismatches"),
                "unscored_fields": len(row.get("unscored_fields", []))}
    return [{"spec": row["spec"], "requested": snapshot(row),
             "source_order": row["source_order"]["mismatches"], "watch": [snapshot(w) for w in row["watch"]]}
            for row in diff["requests"]]


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
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    out = args.output.resolve()
    if not out.is_relative_to(ROOT / "target") or out.exists():
        parser.error("output must be a new isolated directory under target/")
    out.mkdir(parents=True)
    config = out / "replay.json"
    program = "native-feature-replay" if args.feature_case else HISTORY_PROGRAM
    identity = native_input_identity(args.capture)
    if args.feature_case:
        config.write_bytes(args.feature_case.read_bytes())
        case = json.loads(config.read_text(encoding="utf-8"))
        if (case["native_provenance"] != identity["native_provenance_sha256"]
                or case["native_events"] != identity["native_events_sha256"]):
            raise RuntimeError("feature input/native capture provenance mismatch")
    else:
        plan = replay_plan(args.capture)
        if args.trace_feature:
            x, z, feature = args.trace_feature.split(",", 2)
            plan["feature_trace"] = {"source": [int(x), int(z)], "feature": feature}
        validate_replay_plan(plan)
        config.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    tools = tool_sources()
    write_archive(out / "tools.zip", tools)
    exe = out / (program + ".exe")
    if args.reuse_build:
        prior = validate_reuse_build(args.reuse_build, program, bool(args.trace_feature))
        previous_exe = args.reuse_build / (program + ".exe")
        shutil.copy2(previous_exe, exe)
        shutil.copy2(args.reuse_build / "source.zip", out / "source.zip")
        if sha(exe) != prior["binary_sha256"] or sha(out / "source.zip") != prior["source_zip_sha256"]:
            raise RuntimeError("reused build changed during copy")
        provenance = {"reused_build": str(args.reuse_build.resolve()), "reused_provenance_sha256": sha(args.reuse_build / "provenance.json"),
                      **{key: prior[key] for key in ("source_sha256", "binary_sha256", "build_returncode",
                                                    "build_command", "rustc", "feature_trace_version")}}
    else:
        before = sources()
        if hashes(before) != hashes(sources()):
            raise RuntimeError("production files changed during source snapshot; preserve this attempt and retry in a new output")
        source_root = out / "source"
        write_archive(out / "source.zip", before, source_root)
        validate_source_dependencies(out / "source.zip")
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
    if not args.feature_case:
        provenance["binary_contract"] = validate_binary_contract(exe)
    provenance.update({"program": program, "snapshot_boundary_version": REPLAY_SCHEMA if not args.feature_case else None,
                       "replay_schema": REPLAY_SCHEMA if not args.feature_case else None,
                       "observation_contract": OBSERVATION_CONTRACT if not args.feature_case else None,
                       "runner_sha256": hashlib.sha256(tools[OBSERVER + "replay_bcore.py"]).hexdigest(),
                       "comparator_sha256": hashlib.sha256(tools[OBSERVER + "oracle.py"]).hexdigest(),
                       "tool_sha256": hashes(tools), "tools_zip_sha256": sha(out / "tools.zip"),
                       "source_zip_sha256": sha(out / "source.zip"), "binary_sha256": sha(exe),
                       "replay_sha256": sha(config), **identity})
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    result_path = out / ("feature-comparison.json" if args.feature_case else "chunks.jsonl")
    command = [str(exe.resolve()), str(config), str(result_path)]
    env = {**os.environ, "RAYON_NUM_THREADS": "1"}
    with (out / "run.log").open("w", encoding="utf-8") as log:
        subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=args.timeout)
    if hashes(tools) != hashes(tool_sources()):
        raise RuntimeError("observer tools changed during replay; preserved this attempt without comparing")
    if sha(exe) != provenance["binary_sha256"] or sha(config) != provenance["replay_sha256"]:
        raise RuntimeError("binary or replay plan changed during execution")
    if native_input_identity(args.capture) != identity:
        raise RuntimeError("native capture changed during replay")
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
    print(json.dumps({"output": str(out), "requests": comparison_summary(diff)}, indent=2))


if __name__ == "__main__":
    main()
