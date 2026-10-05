"""Isolated, immutable captures of native 26.1 scattered-structure methods."""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "scattered_nbt_support", ROOT / "scripts/jigsaw-reference/capture.py"
)
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)


def sources():
    return [
        *support.probe_sources("JigsawReference"),
        ROOT / "scripts/structure-runtime-reference/StructureRuntimeReference.java",
        ROOT / "scripts/scattered-structure-reference/ScatteredStructureReference.java",
    ]


def workspace_path(path):
    path = path.resolve()
    path.relative_to(ROOT)
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["bytecode", "capture", "verify", "summary"])
    parser.add_argument("--jar", type=Path, default=ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar")
    parser.add_argument("--java", type=Path, default=ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe")
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--build", type=Path, default=ROOT / "target/full-parity-20261003/scattered-native-v1")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--source-snapshot", type=Path, help="Compile the frozen Java sources from an earlier capture")
    parser.add_argument("--compare", type=Path, help="Compare a fresh capture with immutable evidence (the current runner hash may differ)")
    parser.add_argument("--section", default="all")
    parser.add_argument("--classes", nargs="*")
    args = parser.parse_args()
    args.jar, args.java, args.build = map(workspace_path, (args.jar, args.java, args.build))
    if args.output:
        args.output = workspace_path(args.output)
    if args.source_snapshot:
        args.source_snapshot = workspace_path(args.source_snapshot)
    if args.compare:
        args.compare = workspace_path(args.compare)
    if args.mode == "summary":
        data = json.loads(args.output.read_text(encoding="utf-8"))
        if args.section == "starts":
            seen = set()
            for row in data.get("placements", []):
                if row["kind"] in seen:
                    continue
                seen.add(row["kind"])
                print(row["kind"], json.dumps(row["initial"], indent=2))
            return
        if args.section == "totals":
            passes = [p for row in data.get("placements", []) for p in row["passes"]]
            effects = Counter(e[0] for p in passes for e in p["effects"])
            real_starts = [row for row in data.get("admission", []) if row["biome"] == "overworld" and row["starts"]]
            result = {
                "admission_cases": len(data.get("admission", [])),
                "admitted_starts": sum(len(row["starts"]) for row in data.get("admission", [])),
                "real_overworld_starts": [{k: row[k] for k in ("kind", "seed", "chunk", "first_free", "noise_biome")} for row in real_starts],
                "placement_histories": len(data.get("placements", [])),
                "placement_passes": len(passes),
                "accepted_block_writes": sum(p["write_count"] for p in passes),
                "zero_write_passes": sum(p["write_count"] == 0 for p in passes),
                "effects": dict(effects),
                "reference_comparisons": sum(len(row["targets"]) for row in data.get("references", [])),
                "chest_orientation_cases": len(data.get("chest_orientations", [])),
                "container_cases": len(data.get("containers", {}).get("cases", [])),
            }
            print(json.dumps(result, indent=2))
            return
        print(json.dumps({k: len(v) if isinstance(v, list) else v for k, v in data.items()
                          if k not in ("states", "jar_assets", "block_properties")}, indent=2))
        print("catalog", json.dumps(data.get("catalog", []), indent=2))
        print("block_properties", data.get("block_properties", {}).get("state_count"))
        for row in data.get("admission", []):
            print(row["kind"], row["seed"], row["biome"], row["chunk"], row["candidate"],
                  row["first_free"], row["noise_biome"], len(row["starts"]), row["next_i64"])
        for row in data.get("placements", []):
            for passed in row["passes"]:
                print(row["name"], row["kind"], row["seed"], row["orientation"],
                      passed["source"], passed["write_count"], passed["state_count"], passed["next_i64"])
                print("  reference", row["reference_bounds"], "reloaded", passed["reloaded_reference_bounds"],
                      "non-block effects", [e for e in passed["effects"] if e[0] != "block"],
                      "block_entities", passed["block_entities"])
        return

    support.provenance(args.jar, [])
    args.build.mkdir(parents=True, exist_ok=True)
    if args.mode == "bytecode":
        for name in args.classes:
            result = subprocess.run(
                ["javap", "-classpath", str(args.jar), "-c", "-p", name],
                check=True, capture_output=True, text=True,
            )
            path = args.build / (name.rsplit(".", 1)[-1] + ".txt")
            with path.open("x", encoding="utf-8") as stream:
                stream.write(result.stdout)
            print(path)
        return

    if args.output is None:
        parser.error("capture/verify requires --output")
    snapshots = args.build / "probe-sources"
    inputs = sources()
    if args.mode == "verify":
        inputs = [snapshots / source.name for source in inputs]
    elif args.source_snapshot:
        inputs = [args.source_snapshot / source.name for source in inputs]
    header = support.provenance(args.jar, inputs)
    recorded_script = snapshots / Path(__file__).name if args.mode == "verify" else Path(__file__)
    header["capture_script_sha256"] = hashlib.sha256(recorded_script.read_bytes()).hexdigest()
    log = args.build / f"ScatteredStructureReference.{args.section}.stdout.log"
    if args.mode == "capture":
        if args.output.exists() or log.exists():
            raise FileExistsError("Preserve old evidence: select a new output and build directory")
        snapshots.mkdir(exist_ok=True)
        for source in [*inputs, Path(__file__)]:
            with (snapshots / source.name).open("xb") as stream:
                stream.write(source.read_bytes())
        inputs = [snapshots / source.name for source in inputs]
        if support.provenance(args.jar, inputs)["probe_sha256"] != header["probe_sha256"]:
            raise ValueError("Probe source changed while taking the capture snapshot")
        subprocess.run([args.javac, "-d", str(args.build), *map(str, inputs)], check=True)
        libraries = sorted((args.jar.parents[2] / "libraries").rglob("*.jar"))
        classpath = os.pathsep.join(map(str, [args.build, args.jar, *libraries]))
        result = subprocess.run(
            [str(args.java), "-Xmx3G", "-cp", classpath, "ScatteredStructureReference",
             str(args.jar), args.section],
            cwd=args.build, capture_output=True, text=True, timeout=1200,
        )
        log.write_text(result.stdout, encoding="utf-8")
        log.with_suffix(".stderr.log").write_text(result.stderr, encoding="utf-8")
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        if "Serialization errors:" in result.stdout + result.stderr:
            raise ValueError("Native NBT serialization failed; see retained native logs")
    raw = log.read_text(encoding="utf-8")
    marker = "SCATTEREDSTRUCTUREREFERENCE="
    lines = [line.split(marker, 1)[1] for line in raw.splitlines() if marker in line]
    if len(lines) != 1 or "Serialization errors:" in raw:
        raise ValueError(f"Incomplete native recording: {log}")
    data = header | support.decode_fixture(json.loads(lines[0]))
    if args.compare:
        expected = json.loads(args.compare.read_text(encoding="utf-8"))
        captured = dict(data)
        expected.pop("capture_script_sha256", None)
        captured.pop("capture_script_sha256", None)
        if expected != captured:
            raise ValueError("Fresh native capture differs from immutable evidence")
        print(f"Fresh native methods, source hash and JAR match: {args.compare}")
    if args.mode == "verify":
        expected = json.loads(args.output.read_text(encoding="utf-8"))
        if data != expected:
            raise ValueError("Fixture, native recording or source provenance differs")
        print(f"Verified immutable native recording and provenance: {args.output}")
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as stream:
            stream.write(json.dumps(data, separators=(",", ":"), ensure_ascii=False, allow_nan=False) + "\n")
        print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
