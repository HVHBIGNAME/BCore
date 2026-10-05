"""Capture the native falling-block edge updates which can interrupt whole trees."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
JAR = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
JAR_HASH = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    if not args.build_dir.parent.is_dir() or not args.output.parent.is_dir():
        raise ValueError("capture parents must exist")
    if args.output.exists() or args.build_dir.exists():
        raise FileExistsError("native evidence must use new output/build paths")
    if sha(JAR) != JAR_HASH:
        raise ValueError("unexpected Minecraft JAR")
    args.build_dir.mkdir()
    sources = [ROOT / "scripts" / name for name in ("TreeReference.java", "FallingShapeReference.java")]
    hashes = {source.name: sha(source) for source in sources}
    for source in sources:
        (args.build_dir / source.name).write_bytes(source.read_bytes())
    classes = args.build_dir / "classes"
    classes.mkdir()
    compile_command = ["javac", "-d", str(classes), *[str(args.build_dir / p.name) for p in sources]]
    subprocess.run(compile_command, check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [classes.resolve(), JAR, *libraries]))
    command = [str(ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"),
               "-Xmx512m", "-cp", classpath, "FallingShapeReference"]
    run = subprocess.run(command, cwd=args.build_dir, capture_output=True, text=True,
                         encoding="utf-8", timeout=120)
    (args.build_dir / "stdout.log").write_text(run.stdout, encoding="utf-8")
    (args.build_dir / "stderr.log").write_text(run.stderr, encoding="utf-8")
    if run.returncode:
        raise RuntimeError(run.stdout + run.stderr)
    marker = "FALLING_SHAPE_REFERENCE="
    rows = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
    if len(rows) != 1:
        raise ValueError("native output missing or repeated")
    result = {"minecraft": "26.1", "protocol": 775, "jar_sha256": JAR_HASH,
              "sources": hashes, "capture_sha256": sha(Path(__file__)), **json.loads(rows[0])}
    if args.verify and json.loads(args.verify.read_text(encoding="utf-8")) != result:
        raise ValueError("independent native falling-shape capture differs")
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    (args.build_dir / "commands.json").write_text(json.dumps({"compile": compile_command,
        "run": command, "output_sha256": sha(args.output)}, indent=2) + "\n", encoding="utf-8")
    print(f"Captured {len(result['blocks'])} falling block types, {len(result['samples'])} native updates" +
          ("; independent verification passed" if args.verify else ""))


if __name__ == "__main__":
    main()
