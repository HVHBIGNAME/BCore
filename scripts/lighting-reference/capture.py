"""Isolated native 26.1 lighting/environment capture; never rewrites an oracle."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "target/full-parity-20261002/lighting-native"
JAR = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
JAVA = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("inspect", "catalog", "environment", "light", "history", "boundaries", "publish", "updates"))
    parser.add_argument("classes", nargs="*")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    WORK.mkdir(parents=True, exist_ok=True)
    if args.mode == "inspect":
        for name in args.classes:
            result = subprocess.run(["javap", "-classpath", str(JAR), "-p", "-c", name],
                                    capture_output=True, text=True, check=True)
            path = WORK / (name + ".txt")
            path.write_text(result.stdout, encoding="utf-8")
            print(path.relative_to(ROOT))
        return
    build = WORK / "classes"
    build.mkdir(exist_ok=True)
    sources = [ROOT / "scripts" / name for name in (
        "TreeReference.java", "NativeWorldgenRegistries.java", "BaseFeatureReference.java",
        "lighting-reference/LightingReference.java",
    )]
    for source in sources:
        content = source.read_bytes()
        digest = hashlib.sha256(content).hexdigest()
        archive = WORK / "provenance" / digest
        archive.mkdir(parents=True, exist_ok=True)
        saved = archive / source.name
        if not saved.exists():
            saved.write_bytes(content)
    subprocess.run(["javac", "-d", str(build), *map(str, sources)], check=True)
    classpath = os.pathsep.join(map(str, [build, JAR, *sorted(
        (ROOT / "target/vanilla-775/libraries").rglob("*.jar"))]))
    result = subprocess.run([str(JAVA), "-Xmx2G", "-cp", classpath,
                             "LightingReference", args.mode, str(JAR)],
                            cwd=WORK, capture_output=True, text=True, timeout=600)
    (WORK / (args.mode + ".log")).write_text(result.stdout + result.stderr, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    records = [line.split("LIGHTING_REFERENCE=", 1)[1] for line in result.stdout.splitlines()
               if "LIGHTING_REFERENCE=" in line]
    if len(records) != 1:
        raise RuntimeError(result.stdout + result.stderr)
    reference = {
        "minecraft": "26.1", "protocol": 775,
        "jar_sha256": hashlib.sha256(JAR.read_bytes()).hexdigest(),
        "probe_sources": {str(s.relative_to(ROOT)): hashlib.sha256(s.read_bytes()).hexdigest()
                          for s in sources},
        **json.loads(records[0]),
    }
    if args.verify:
        old = json.loads(args.verify.read_text(encoding="utf-8"))
        # Source evolves as new independent modes are added. Values are immutable.
        assert {k: v for k, v in old.items() if k != "probe_sources"} == {
            k: v for k, v in reference.items() if k != "probe_sources"}, "native values changed"
        (WORK / (args.mode + ".verified.json")).write_text(json.dumps({
            "fixture_sha256": hashlib.sha256(args.verify.read_bytes()).hexdigest(),
            "verification_probe_sources": reference["probe_sources"],
            "jar_sha256": reference["jar_sha256"], "values_equal": True,
        }, indent=2) + "\n", encoding="utf-8")
        print(f"Verified native values: {args.verify}")
    if args.output:
        if args.output.exists():
            raise FileExistsError(f"Refusing to overwrite native fixture: {args.output}")
        args.output.write_text(json.dumps(reference, indent=2) + "\n", encoding="utf-8")
        print(f"Captured {args.mode}: {args.output}")


if __name__ == "__main__":
    main()
