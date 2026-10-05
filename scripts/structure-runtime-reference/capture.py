"""Isolated 26.1 structure runtime differential capture (never overwrites old fixtures)."""
from __future__ import annotations

import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("jigsaw_capture", ROOT / "scripts/jigsaw-reference/capture.py")
jigsaw = importlib.util.module_from_spec(spec)
spec.loader.exec_module(jigsaw)


def sources(section="all"):
    result = [*jigsaw.probe_sources("JigsawReference"),
            ROOT / "scripts/structure-runtime-reference/StructureRuntimeReference.java"]
    if section == "trail":
        result.append(ROOT / "scripts/structure-runtime-reference/TrailAdmissionReference.java")
    elif section == "brushable":
        result.append(ROOT / "scripts/structure-runtime-reference/BrushableRuntimeReference.java")
    elif section == "trial-block-entities":
        result.append(ROOT / "scripts/structure-runtime-reference/TrialBlockEntityRuntimeReference.java")
    elif section == "trial":
        result.append(ROOT / "scripts/structure-runtime-reference/TrialAdmissionReference.java")
    elif section == "biome-fill":
        result.append(ROOT / "scripts/structure-runtime-reference/BiomeFillReference.java")
    elif section == "decorated-pot":
        result.extend(ROOT / "scripts/structure-runtime-reference" / name for name in
                      ["BrushableRuntimeReference.java", "DecoratedPotRuntimeReference.java"])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["capture", "verify", "bytecode", "summary"])
    parser.add_argument("--jar", type=Path, default=ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar")
    parser.add_argument("--java", type=Path, default=ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe")
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--build", type=Path, default=ROOT / "target/full-parity-20261002/structure-runtime-native")
    parser.add_argument("--output", type=Path, default=ROOT / "crates/bcore-worldgen/data/structure_runtime_fresh_light_26_1.json")
    parser.add_argument("--classes", nargs="*")
    parser.add_argument("--section", default="all")
    parser.add_argument("--case", action="append", default=[], help="filter named block-entity cases in summary mode")
    args = parser.parse_args()
    args.jar, args.build = args.jar.resolve(), args.build.resolve()
    if args.mode == "summary":
        data = json.loads(args.output.read_text(encoding="utf-8"))
        print(json.dumps({k: len(v) if isinstance(v, list) else v for k, v in data.items()}, indent=2))
        for row in data.get("admission", []):
            print(row["set"], row["seed"], row["biome"], row["chunk"],
                  [(s["structure"], s["pieces"]) for s in row["starts"]])
        for row in data.get("block_entity_policies", []):
            print(row["block"], row["type_id"], row["update_owner"], json.dumps(row["update"]))
        for row in data.get("metadata", []):
            print(row)
        for row in data.get("block_entity_loads", []):
            if not args.case or any(name in row["name"] for name in args.case):
                print(row["name"], json.dumps({key: row[key] for key in ("load", "full", "update")}))
        return
    jigsaw.provenance(args.jar, [])
    args.build.mkdir(parents=True, exist_ok=True)
    if args.mode == "bytecode":
        for name in args.classes:
            result = subprocess.run(["javap", "-classpath", str(args.jar), "-c", "-p", name],
                                    check=True, capture_output=True, text=True)
            path = args.build / (name.rsplit(".", 1)[-1] + ".txt")
            path.write_text(result.stdout, encoding="utf-8")
            print(path)
        return
    probe = {"trail": "TrailAdmissionReference", "trial": "TrialAdmissionReference", "brushable": "BrushableRuntimeReference",
             "trial-block-entities": "TrialBlockEntityRuntimeReference", "biome-fill": "BiomeFillReference",
             "decorated-pot": "DecoratedPotRuntimeReference"}.get(args.section, "StructureRuntimeReference")
    inputs = sources(args.section)
    header = jigsaw.provenance(args.jar, inputs)
    header["capture_script_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    log = args.build / f"{probe}.{args.section}.stdout.log"
    if args.mode == "capture":
        if args.output.exists():
            raise FileExistsError(f"Preserve existing native evidence: choose a new --output ({args.output})")
        source_snapshot = args.build / "probe-sources"
        source_snapshot.mkdir(exist_ok=True)
        for source in [*inputs, Path(__file__)]:
            (source_snapshot / source.name).write_bytes(source.read_bytes())
        subprocess.run([args.javac, "-d", str(args.build), *map(str, inputs)], check=True)
        libraries = sorted((args.jar.parents[2] / "libraries").rglob("*.jar"))
        classpath = os.pathsep.join(map(str, [args.build, args.jar, *libraries]))
        result = subprocess.run([str(args.java.resolve()), "-Xmx3G", "-cp", classpath,
                                 probe, str(args.jar), args.section],
                                cwd=args.build, capture_output=True, text=True, timeout=1200)
        log.write_text(result.stdout, encoding="utf-8")
        log.with_name(log.name.replace("stdout", "stderr")).write_text(result.stderr, encoding="utf-8")
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        if "Serialization errors:" in result.stdout + result.stderr:
            raise ValueError("Native NBT serialization failed; preserve log and fix probe lifecycle")
    raw = log.read_text(encoding="utf-8")
    marker = "STRUCTURERUNTIMEREFERENCE="
    lines = [line.split(marker, 1)[1] for line in raw.splitlines() if marker in line]
    if len(lines) != 1 or "Serialization errors:" in raw:
        raise ValueError(f"Not a complete native recording: {log}")
    data = header | jigsaw.decode_fixture(json.loads(lines[0]))
    if args.mode == "verify":
        expected = json.loads(args.output.read_text(encoding="utf-8"))
        if data != expected:
            raise ValueError("Fixture, probe provenance or raw recording differ")
        print(f"Verified {args.output} against unmodified native recording and provenance")
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(data, separators=(",", ":"), ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")
        print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
