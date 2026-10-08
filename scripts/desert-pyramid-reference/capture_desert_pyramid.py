"""Immutable native 26.1 desert-pyramid evidence and pristine replay snapshots."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("pyramid_nbt", ROOT / "scripts/jigsaw-reference/capture.py")
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, data):
    with path.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(data, separators=(",", ":"), ensure_ascii=False, allow_nan=False) + "\n")


def sources():
    return [*support.probe_sources("JigsawReference"),
            ROOT / "scripts/structure-runtime-reference/StructureRuntimeReference.java",
            ROOT / "scripts/scattered-structure-reference/ScatteredStructureReference.java",
            HERE / "DesertPyramidReference.java"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("capture", "bytecode", "freeze-base", "package", "summary"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--section", choices=("all", "admission", "placement"), default="all")
    parser.add_argument("--compare", type=Path)
    parser.add_argument("--source-snapshot", type=Path)
    parser.add_argument("--fixture", type=Path)
    args = parser.parse_args()
    out = args.output.resolve()
    if args.mode == "summary":
        data = json.loads(out.read_text(encoding="utf-8"))
        passes = [p for row in data.get("placements", []) for p in row["passes"]]
        print(json.dumps({"sha256": sha(out), "probe_sha256": data["probe_sha256"],
            "admission_cases": len(data.get("admission", [])),
            "real_starts": [r for r in data.get("admission", []) if r["biome"] == "overworld" and r["starts"]],
            "histories": len(data.get("placements", [])), "passes": len(passes),
            "writes": sum(p["write_count"] for p in passes),
            "references": sum(len(r["targets"]) for r in data.get("references", [])),
            "orientations": sorted({r["orientation"] for r in data.get("placements", [])}),
            "catalog": data.get("catalog")}, indent=2))
        return
    if args.mode == "package":
        if args.fixture is None:
            parser.error("package requires --fixture (verified native fixture)")
        data = json.loads(args.fixture.read_text(encoding="utf-8"))
        out.relative_to(ROOT / "crates/bcore-worldgen/data")
        write_json(out, data)
        catalog = {k: data[k] for k in ("minecraft", "jar_sha256", "probe_sha256", "catalog")}
        catalog["fixture_sha256"] = sha(args.fixture)
        write_json(out.with_name("desert_pyramid_catalog_26_1.json"), catalog)
        print(json.dumps({"fixture": str(out), "sha256": sha(out)}))
        return
    out.relative_to(ROOT / "target")
    if out.exists():
        raise FileExistsError(f"preserve evidence; select a new directory: {out}")
    out.mkdir(parents=True)
    if args.mode == "freeze-base":
        sys.path.insert(0, str(ROOT / "scripts/native-generation-reference"))
        from replay_bcore import sources as replay_sources, hashes
        files = replay_sources()
        if hashes(files) != hashes(replay_sources()):
            raise RuntimeError("source changed during freeze")
        for name, content in files.items():
            target = out / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        # The frozen replay runner imports the unmodified Python oracle/runner.
        for file in (ROOT / "scripts/native-generation-reference").glob("*.py"):
            (out / "scripts/native-generation-reference" / file.name).write_bytes(file.read_bytes())
        (out / "target").mkdir()
        write_json(out / "freeze.json", {"base": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "sources": hashes(files), "runner_sha256": sha(Path(__file__))})
        print(out)
        return
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    support.provenance(jar, [])
    if args.mode == "bytecode":
        classes = ["structure.structures.DesertPyramidStructure", "structure.structures.DesertPyramidPiece",
                   "structure.ScatteredFeaturePiece", "structure.SinglePieceStructure", "structure.StructurePiece",
                   "structure.StructureStart", "LegacyRandomSource$LegacyPositionalRandomFactory", "SingleThreadedRandomSource"]
        for name in classes:
            result = subprocess.run(["javap", "-classpath", str(jar), "-c", "-p", "net.minecraft.world.level.levelgen." + name],
                                    check=True, capture_output=True, text=True)
            (out / (name.rsplit(".", 1)[-1] + ".txt")).write_text(result.stdout, encoding="utf-8")
        write_json(out / "provenance.json", {"jar_sha256": sha(jar), "files": {p.name: sha(p) for p in out.glob("*.txt")}})
        print(out)
        return
    snapshot = out / "probe-sources"
    snapshot.mkdir()
    inputs = sources()
    if args.source_snapshot:
        inputs = [args.source_snapshot / p.name for p in inputs]
    header = support.provenance(jar, inputs)
    for file in [*inputs, Path(__file__), ROOT / "scripts/jigsaw-reference/capture.py"]:
        (snapshot / file.name).write_bytes(file.read_bytes())
    inputs = [snapshot / p.name for p in inputs]
    if support.provenance(jar, inputs)["probe_sha256"] != header["probe_sha256"]:
        raise RuntimeError("probe changed during snapshot")
    header["capture_script_sha256"] = sha(Path(__file__))
    header["nbt_decoder_sha256"] = sha(snapshot / "capture.py")
    command = ["javac", "-d", str(out), *map(str, inputs)]
    subprocess.run(command, check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [out, jar, *libraries]))
    command = [str(java), "-Xmx3G", "-XX:ActiveProcessorCount=2", "-cp", classpath,
               "DesertPyramidReference", str(jar), args.section]
    write_json(out / "provenance.json", {**header, "command": command,
        "java": subprocess.run([str(java), "-version"], capture_output=True, text=True, check=True).stderr,
        "sources": {p.name: sha(p) for p in snapshot.iterdir()}})
    result = subprocess.run(command, cwd=out, capture_output=True, text=True, timeout=1800)
    (out / "stdout.log").write_text(result.stdout, encoding="utf-8")
    (out / "stderr.log").write_text(result.stderr, encoding="utf-8")
    if result.returncode or "Serialization errors:" in result.stdout + result.stderr:
        raise RuntimeError(f"native failure; preserved logs at {out}")
    marker = "DESERTPYRAMIDREFERENCE="
    rows = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(rows) != 1:
        raise RuntimeError("missing complete native result")
    data = header | support.decode_fixture(json.loads(rows[0]))
    write_json(out / "fixture.json", data)
    if args.compare:
        expected = json.loads(args.compare.read_text(encoding="utf-8"))
        comparable = dict(data)
        for item in (expected, comparable):
            item.pop("capture_script_sha256", None)
        if expected != comparable:
            raise ValueError("independent native repetition differs")
    print(json.dumps({"output": str(out), "fixture_sha256": sha(out / "fixture.json"),
        "probe_sha256": header["probe_sha256"], "repeat_matches": args.compare is not None}))


if __name__ == "__main__":
    main()
