"""Record native stair corner updates without changing previous shape evidence."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from capture_falling_shape_reference import ROOT, JAR, JAR_HASH, sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    if not args.build_dir.parent.is_dir() or not args.output.parent.is_dir():
        raise ValueError("capture parents must exist")
    if args.output.exists() or args.build_dir.exists():
        raise FileExistsError("preserve native evidence: use new capture paths")
    if sha(JAR) != JAR_HASH:
        raise ValueError("unexpected Minecraft JAR")
    args.build_dir.mkdir()
    sources = [ROOT / "scripts" / name for name in
               ("TreeReference.java", "FallingShapeReference.java", "StairShapeReference.java")]
    hashes = {p.name: sha(p) for p in sources}
    for source in [*sources, Path(__file__), ROOT / "scripts/capture_falling_shape_reference.py"]:
        (args.build_dir / source.name).write_bytes(source.read_bytes())
    classes = args.build_dir / "classes"
    classes.mkdir()
    compile_command = ["javac", "-d", str(classes), *[str(args.build_dir / p.name) for p in sources]]
    subprocess.run(compile_command, check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [classes.resolve(), JAR, *libraries]))
    command = [str(ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"),
               "-Xmx1G", "-cp", classpath, "StairShapeReference"]
    run = subprocess.run(command, cwd=args.build_dir, capture_output=True, text=True,
                         encoding="utf-8", timeout=240)
    (args.build_dir / "stdout.log").write_text(run.stdout, encoding="utf-8")
    (args.build_dir / "stderr.log").write_text(run.stderr, encoding="utf-8")
    if run.returncode:
        raise RuntimeError(run.stdout + run.stderr)
    marker = "STAIR_SHAPE_REFERENCE="
    lines = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
    if len(lines) != 1:
        raise ValueError("missing or duplicate native payload")
    result = {"minecraft": "26.1", "protocol": 775, "jar_sha256": JAR_HASH,
              "sources": hashes, "capture_sha256": sha(Path(__file__)),
              "capture_helper_sha256": sha(ROOT / "scripts/capture_falling_shape_reference.py"),
              **json.loads(lines[0])}
    if args.verify and json.loads(args.verify.read_text(encoding="utf-8")) != result:
        raise ValueError("independent native stair-shape capture differs")
    args.output.write_text(json.dumps(result, separators=(",", ":")) + "\n", encoding="utf-8")
    (args.build_dir / "commands.json").write_text(json.dumps({"compile": compile_command,
        "run": command, "output_sha256": sha(args.output)}, indent=2) + "\n", encoding="utf-8")
    print(f"Captured {len(result['blocks'])} stair types, {len(result['samples'])} native updates" +
          ("; independent verification passed" if args.verify else ""))


if __name__ == "__main__":
    main()
