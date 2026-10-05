"""Pinned, isolated BlockAge/Blackstone native probe and recording verification."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("jigsaw_capture", ROOT / "scripts/jigsaw-reference/capture.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["capture", "verify"])
    parser.add_argument("--build", type=Path, default=ROOT / "target/full-parity-20261002/structure-processors-native")
    parser.add_argument("--output", type=Path, default=ROOT / "crates/bcore-worldgen/data/structure_processors_extra_26_1.json")
    args = parser.parse_args()
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    sources = [*base.probe_sources("JigsawReference")[:-1], Path(__file__).with_name("StructureProcessorReference.java")]
    header = base.provenance(jar, sources)
    header["capture_script_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    build = args.build.resolve()
    log = build / "StructureProcessorReference.stdout.log"
    if args.mode == "capture":
        if args.output.exists():
            raise FileExistsError(f"Preserve native evidence; choose a new --output: {args.output}")
        build.mkdir(parents=True, exist_ok=True)
        snapshot = build / "probe-sources"
        snapshot.mkdir(exist_ok=True)
        for source in [*sources, Path(__file__)]:
            (snapshot / source.name).write_bytes(source.read_bytes())
        subprocess.run(["javac", "-d", str(build), *map(str, sources)], check=True)
        libraries = sorted((jar.parents[2] / "libraries").rglob("*.jar"))
        result = subprocess.run([str(java), "-Xmx2G", "-cp", os.pathsep.join(map(str, [build, jar, *libraries])),
                                 "StructureProcessorReference"], cwd=build, capture_output=True, text=True, timeout=600)
        log.write_text(result.stdout, encoding="utf-8")
        log.with_name("StructureProcessorReference.stderr.log").write_text(result.stderr, encoding="utf-8")
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
    raw = log.read_text(encoding="utf-8")
    marker = "STRUCTUREPROCESSORREFERENCE="
    rows = [line.split(marker, 1)[1] for line in raw.splitlines() if marker in line]
    if len(rows) != 1 or "Serialization errors:" in raw:
        raise ValueError("Incomplete native recording")
    data = header | base.decode_fixture(json.loads(rows[0]))
    if args.mode == "verify":
        if json.loads(args.output.read_text(encoding="utf-8")) != data:
            raise ValueError("Recording, provenance or fixture mismatch")
        print(f"Verified {len(data['cases'])} exhaustive native batches")
    else:
        args.output.write_text(json.dumps(data, separators=(",", ":"), allow_nan=False) + "\n", encoding="utf-8")
        print(f"Wrote {len(data['cases'])} batches / {sum(len(c['states']) for c in data['cases'])} native processor calls to {args.output}")


if __name__ == "__main__":
    main()
