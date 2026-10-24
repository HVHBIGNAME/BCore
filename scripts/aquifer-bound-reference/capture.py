"""Capture immutable aquifer-bound witnesses from the pinned, unmodified 26.1 JAR."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    jar_hash = hashlib.sha256(jar.read_bytes()).hexdigest()
    if jar_hash != JAR_SHA256:
        raise ValueError(f"wrong native JAR: {jar_hash}")
    sources = [ROOT / path for path in (
        "scripts/TreeReference.java", "scripts/NativeWorldgenRegistries.java",
        "scripts/jigsaw-reference/JigsawSupport.java",
        "scripts/density-materials-reference/DensityMaterialsReference.java",
        "scripts/aquifer-bound-reference/AquiferBoundReference.java",
    )]
    build = args.build.resolve()
    build.mkdir(parents=True, exist_ok=False)
    (build / "sources").mkdir()
    for source in sources:
        (build / "sources" / source.name).write_bytes(source.read_bytes())
    frozen = [build / "sources" / source.name for source in sources]
    compile_result = subprocess.run(["javac", "-d", str(build), *map(str, frozen)],
                                    capture_output=True, text=True)
    (build / "javac.stdout.log").write_text(compile_result.stdout, encoding="utf-8")
    (build / "javac.stderr.log").write_text(compile_result.stderr, encoding="utf-8")
    compile_result.check_returncode()
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [build, jar, *libraries]))
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    result = subprocess.run([str(java), "-Xmx1G", "-cp", classpath, "AquiferBoundReference"],
                            cwd=build, capture_output=True, text=True, timeout=900)
    (build / "native.stdout.log").write_text(result.stdout, encoding="utf-8")
    (build / "native.stderr.log").write_text(result.stderr, encoding="utf-8")
    result.check_returncode()
    marker = "AQUIFER_BOUND_REFERENCE="
    records = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise ValueError(f"expected one native result; see {build}")
    data = {"minecraft": "26.1", "protocol": 775, "jar_sha256": jar_hash,
            "sources": {p.relative_to(ROOT).as_posix(): hashlib.sha256(snapshot.read_bytes()).hexdigest()
                        for p, snapshot in zip(sources, frozen)}, **json.loads(records[0])}
    if args.verify:
        if json.loads(args.verify.read_text(encoding="utf-8")) != data:
            raise ValueError(f"native recapture differs from {args.verify}")
        print(f"Verified {args.verify}")
        return
    output = args.output or build / "aquifer_bound_26_1.json"
    with output.open("x", encoding="utf-8") as stream:
        json.dump(data, stream, indent=2, allow_nan=False)
        stream.write("\n")
    print(f"Captured {output}; sha256={hashlib.sha256(output.read_bytes()).hexdigest()}")
    for case in data["cases"]:
        print(case["id"], "H=", case["highest"], "T=", case["threshold"],
              "inside/outside=", case["inside_max"], case["outside_max"],
              "history=", len(case.get("history", [])))


if __name__ == "__main__":
    main()
