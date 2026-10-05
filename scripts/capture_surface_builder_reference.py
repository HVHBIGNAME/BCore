"""Capture the complete 26.1 native surface pass in an isolated Java target."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PINNED_JAR = "target/vanilla-775/versions/26.1/server-26.1.jar"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java", required=True, type=Path)
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--output", type=Path, default=ROOT / "crates/bcore-worldgen/data/surface_builder_26_1.json")
    parser.add_argument("--blocks-output", type=Path, default=ROOT / "crates/bcore-worldgen/data/surface_builder_blocks_26_1.json")
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    jar = ROOT / PINNED_JAR
    provenance = json.loads((ROOT / "crates/bcore-worldgen/data/carvers_26_1.json").read_text(encoding="utf-8"))
    jar_hash = hashlib.sha256(jar.read_bytes()).hexdigest()
    if provenance["minecraft"] != "26.1" or provenance["jar_sha256"] != jar_hash:
        raise ValueError("surface oracle must use the pinned 26.1 server JAR")
    build = ROOT / "target/surface-builder-reference"
    build.mkdir(exist_ok=True)
    sources = [ROOT / "scripts" / name for name in (
        "TreeReference.java", "NativeWorldgenRegistries.java", "SurfaceBuilderReference.java")]
    subprocess.run([args.javac, "-d", str(build), *map(str, sources)], check=True)
    classpath = os.pathsep.join(map(str, [build, jar, *sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))]))
    result = subprocess.run([str(args.java.resolve()), "-Xmx1g", "-cp", classpath, "SurfaceBuilderReference"],
                            cwd=build, capture_output=True, text=True, timeout=600)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    marker = "SURFACE_BUILDER_REFERENCE="
    records = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise ValueError(f"expected one native surface result: {result.stdout}")
    reference = {"minecraft": "26.1", "jar_sha256": jar_hash,
                 "probe_sha256": hashlib.sha256(b"".join(source.read_bytes() for source in sources)).hexdigest()}
    reference.update(json.loads(records[0]))
    blocks = {key: reference[key] for key in ("minecraft", "jar_sha256", "probe_sha256")}
    blocks.update(reference.pop("block_predicates"))
    for path, data in ((args.output, reference), (args.blocks_output, blocks)):
        if args.verify:
            if json.loads(path.read_text(encoding="utf-8")) != data:
                raise ValueError(f"native capture differs from {path}")
        else:
            path.write_text(json.dumps(data, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"{'Verified' if args.verify else 'Captured'} {len(reference['samples'])} complete native surface slabs "
          f"({len(reference['samples']) * 384 * 256:,} voxels; "
          f"{sum(s['rule_calls'] for s in reference['samples']):,} ordered rule contexts), "
          "including postprocessing and world-surface heights")


if __name__ == "__main__":
    main()
