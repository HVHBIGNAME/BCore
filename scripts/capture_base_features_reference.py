"""Capture the scoped placement/base-feature oracles from the pinned 26.1 JAR."""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", choices=("catalog", "base", "placement"))
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    vanilla = ROOT / "target/vanilla-775"
    jar = vanilla / "versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    build = ROOT / "target/base-features-ref"
    build.mkdir(exist_ok=True)
    sources = [ROOT / "scripts" / name for name in (
        "TreeReference.java", "NativeWorldgenRegistries.java", "BaseFeatureReference.java",
    )]
    if args.probe == "placement":
        sources.append(ROOT / "scripts/PlacementReference.java")
    subprocess.run(["javac", "-d", str(build), *map(str, sources)], check=True)
    classpath = os.pathsep.join(map(str, [build, jar, *sorted((vanilla / "libraries").rglob("*.jar"))]))
    result = subprocess.run(
        [str(java), "-Xmx1G", "-cp", classpath, sources[-1].stem, args.probe, str(jar)],
        cwd=build, capture_output=True, text=True, timeout=300,
    )
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    marker = "BASE_FEATURE_REFERENCE="
    records = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise RuntimeError(result.stdout + result.stderr)
    reference = {
        "minecraft": "26.1",
        "jar_sha256": hashlib.sha256(jar.read_bytes()).hexdigest(),
        "probe_sha256": hashlib.sha256(b"".join(source.read_bytes() for source in sources)).hexdigest(),
        **json.loads(records[0]),
    }
    if args.verify and reference != json.loads(args.verify.read_text(encoding="utf-8")):
        raise ValueError(f"native capture differs from {args.verify}")
    args.output.write_text(json.dumps(reference, indent=2) + "\n", encoding="utf-8")
    print(f"{'Verified' if args.verify else 'Captured'} {args.probe}: {args.output}")
    if args.probe == "catalog":
        for section in ("configured_feature", "placed_feature"):
            entries = reference[section].values()
            types = Counter(v["type"] for v in entries) if section == "configured_feature" else Counter(
                modifier["type"] for value in entries for modifier in value["placement"]
            )
            print(f"{section}: {dict(sorted(types.items()))}")
    elif args.probe == "placement":
        print(f"Overworld kernels: {', '.join(reference['kernel_roots'])}")
    elif args.probe == "base":
        print(f"{len(reference['samples'])} features, {len(reference['survival'])} survival cases, {len(reference['providers'])} providers")


if __name__ == "__main__":
    main()
