"""Capture actual 26.1 cave features in a controlled, fail-closed Java world.

Only this probe's build directory and explicitly requested fixture are written.
The Java probe invokes native configured features; no feature algorithm is
implemented in the reference generator.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
JAR = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
JAVA = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
BUILD = ROOT / "target/cave-feature-reference"


def inventory():
    with zipfile.ZipFile(JAR) as jar:
        for path in jar.namelist():
            if "/worldgen/configured_feature/" in path and any(
                word in path for word in ("dripstone", "lush", "moss", "root", "cave_vine", "lichen", "dripleaf")
            ):
                print(path)
                print(jar.read(path).decode())


def normalize(value):
    if isinstance(value, dict):
        return {key: normalize(item) for key, item in sorted(value.items())}
    if isinstance(value, list):
        return [normalize(item) for item in value]
    if isinstance(value, float) and value.is_integer():
        return int(value)
    return value


def format_json(value, level=0):
    indent = "  " * level
    child_indent = indent + "  "
    if isinstance(value, dict) and value:
        rows = [child_indent + json.dumps(key) + ": " + format_json(item, level + 1) for key, item in value.items()]
        return "{\n" + ",\n".join(rows) + "\n" + indent + "}"
    if isinstance(value, list) and value and (len(value) > 16 or any(isinstance(item, (dict, list)) for item in value)):
        rows = [child_indent + format_json(item, level + 1) for item in value]
        return "[\n" + ",\n".join(rows) + "\n" + indent + "]"
    return json.dumps(value, allow_nan=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", action="store_true")
    parser.add_argument("--javap", nargs="+", metavar="CLASS", help="write bytecode/bootstraps under this probe's build directory")
    parser.add_argument("--mode", choices=("blocks", "features"), default="features")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    if args.inventory:
        inventory()
        return
    if args.javap:
        BUILD.mkdir(exist_ok=True)
        for name in args.javap:
            run = subprocess.run(["javap", "-classpath", str(JAR), "-c", "-p", "-v", name], check=True, capture_output=True, text=True)
            target = BUILD / (name.rsplit(".", 1)[-1] + ".javap.txt")
            target.write_text(run.stdout, encoding="utf-8")
            print(target)
        return
    if not args.output:
        parser.error("--output is required for capture")
    if not args.output.parent.is_dir():
        parser.error("output parent must exist")
    BUILD.mkdir(exist_ok=True)
    sources = [ROOT / "scripts" / name for name in (
        "TreeReference.java", "NativeWorldgenRegistries.java", "CaveFeatureReference.java"
    )]
    subprocess.run(["javac", "-d", str(BUILD), *map(str, sources)], check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [BUILD, JAR, *libraries]))
    run = subprocess.run(
        [str(JAVA), "-Xmx1g", "-cp", classpath, "CaveFeatureReference", args.mode],
        cwd=BUILD, capture_output=True, text=True, timeout=240,
    )
    if run.returncode:
        raise RuntimeError(run.stdout + run.stderr)
    marker = "CAVE_FEATURE_REFERENCE="
    records = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise ValueError(f"Expected one capture: {run.stdout}")
    result = {
        "minecraft": "26.1",
        "jar_sha256": hashlib.sha256(JAR.read_bytes()).hexdigest(),
        "probe_sha256": hashlib.sha256(b"".join(s.read_bytes() for s in sources)).hexdigest(),
        **normalize(json.loads(records[0])),
    }
    if args.verify and result != json.loads(args.verify.read_text(encoding="utf-8")):
        raise ValueError(f"Native result differs from {args.verify}")
    args.output.write_text(format_json(result) + "\n", encoding="utf-8")
    print(f"Captured {args.mode} to {args.output}")


if __name__ == "__main__":
    main()
