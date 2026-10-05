"""Isolated, provenance-checked Minecraft 26.1 density/materials oracle.

Shared native helpers are compiled read-only into this probe's own build. The
probe invokes the original JAR implementations; it never patches native code.
Existing fixtures are immutable: --verify compares without writing them.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
PINNED_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--section", choices=("materials", "beard", "noise", "barriers", "describe"), required=True)
    parser.add_argument("--java", type=Path, default=ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe")
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--build", type=Path, default=ROOT / "target/full-parity-20261002/native-density-materials")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    jar_hash = hashlib.sha256(jar.read_bytes()).hexdigest()
    if jar_hash != PINNED_SHA256:
        raise ValueError(f"wrong native JAR: {jar_hash}")
    sources = [ROOT / path for path in (
        "scripts/TreeReference.java", "scripts/NativeWorldgenRegistries.java",
        "scripts/jigsaw-reference/JigsawSupport.java",
        "scripts/density-materials-reference/DensityMaterialsReference.java",
    )]
    probe_class = "DensityMaterialsReference"
    if args.section == "barriers":
        probe_class = "DensityMaterialBarrierReference"
        sources.append(ROOT / "scripts/density-materials-reference/DensityMaterialBarrierReference.java")
    build = args.build.resolve()
    build.mkdir(parents=True, exist_ok=True)
    subprocess.run([args.javac, "-d", str(build), *map(str, sources)], check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [build, jar, *libraries]))
    result = subprocess.run([str(args.java.resolve()), "-Xmx2G", "-cp", classpath,
                             probe_class, args.section],
                            cwd=build, capture_output=True, text=True, timeout=900)
    (build / f"{args.section}.stdout.log").write_text(result.stdout, encoding="utf-8")
    (build / f"{args.section}.stderr.log").write_text(result.stderr, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    marker = "DENSITY_MATERIALS_REFERENCE="
    records = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise ValueError(f"expected one native result; see {build}")
    data = {"minecraft": "26.1", "protocol": 775, "jar_sha256": jar_hash,
            "probe_sha256": hashlib.sha256(b"".join(p.read_bytes() for p in sources)).hexdigest(),
            "sources": {str(p.relative_to(ROOT)).replace(os.sep, "/"): hashlib.sha256(p.read_bytes()).hexdigest()
                        for p in sources},
            "section": args.section}
    data.update(json.loads(records[0]))
    with zipfile.ZipFile(jar) as archive:
        paths = ["data/minecraft/worldgen/noise_settings/overworld.json"]
        paths += [f"data/minecraft/worldgen/noise/ore_{name}.json" for name in ("veininess", "vein_a", "vein_b", "gap")]
        data["jar_assets"] = {path: {"sha256": hashlib.sha256(archive.read(path)).hexdigest(),
                                     "value": json.loads(archive.read(path))} for path in paths}
    if args.section == "describe":
        print(json.dumps({"states": data["states"], "noise_router": {
            key: data["jar_assets"][paths[0]]["value"]["noise_router"][key]
            for key in ("vein_toggle", "vein_ridged", "vein_gap")}}, indent=2))
        return
    if args.verify:
        expected = json.loads(args.verify.read_text(encoding="utf-8"))
        if expected != data:
            raise ValueError(f"native recapture differs from {args.verify}; fixture was not changed")
        print(f"Verified {args.section}: {args.verify}")
        return
    output = args.output or build / f"{args.section}_26_1.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    # Refuse silent replacement of a previously captured oracle.
    with output.open("x", encoding="utf-8") as stream:
        json.dump(data, stream, separators=(",", ":"), ensure_ascii=False, allow_nan=False)
        stream.write("\n")
    print(f"Captured {args.section}: {output}")


if __name__ == "__main__":
    main()
