"""Capture actual 26.1 light packet classification/bytes without implementing a writer."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
JAR = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
JAR_HASH = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    if not args.build_dir.parent.is_dir() or not args.output.parent.is_dir():
        raise ValueError("capture parents must exist")
    if args.output.exists() or args.build_dir.exists():
        raise FileExistsError("use new output/build paths; native evidence is never overwritten")
    if sha(JAR.read_bytes()) != JAR_HASH:
        raise ValueError("unexpected Minecraft JAR")
    args.build_dir.mkdir()
    sources = [ROOT / "scripts" / name for name in ("TreeReference.java", "ChunkLightPacketReference.java")]
    source_bytes = {str(source.relative_to(ROOT)): source.read_bytes() for source in sources}
    for name, data in source_bytes.items():
        (args.build_dir / Path(name).name).write_bytes(data)
    classes = args.build_dir / "classes"
    classes.mkdir()
    command = ["javac", "-encoding", "UTF-8", "-d", str(classes),
               *[str(args.build_dir / source.name) for source in sources]]
    subprocess.run(command, check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [classes.resolve(), JAR, *libraries]))
    run_command = [str(ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"),
                   "-Xmx512m", "-cp", classpath, "ChunkLightPacketReference"]
    run = subprocess.run(run_command, cwd=args.build_dir, capture_output=True,
                         text=True, encoding="utf-8", timeout=120)
    (args.build_dir / "stdout.log").write_text(run.stdout, encoding="utf-8")
    (args.build_dir / "stderr.log").write_text(run.stderr, encoding="utf-8")
    if run.returncode:
        raise RuntimeError(run.stdout + run.stderr)
    marker = "CHUNK_LIGHT_PACKET_REFERENCE="
    records = [line.split(marker, 1)[1] for line in run.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise RuntimeError("missing or repeated native output")
    result = {
        "minecraft": "26.1", "protocol": 775, "jar_sha256": JAR_HASH,
        "sources": {name: sha(data) for name, data in source_bytes.items()},
        "capture_sha256": sha(Path(__file__).read_bytes()),
        "native_entry_point": "ClientboundLightUpdatePacketData constructor + write(FriendlyByteBuf)",
        "samples": json.loads(records[0]),
    }
    if args.verify and json.loads(args.verify.read_text(encoding="utf-8")) != result:
        raise ValueError("independent native packet capture differs")
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    (args.build_dir / "commands.json").write_text(json.dumps({
        "compile": command, "run": run_command, "output_sha256": sha(args.output.read_bytes()),
    }, indent=2) + "\n", encoding="utf-8")
    print(f"Captured {len(result['samples'])} native light-packet cases" + ("; independently verified" if args.verify else ""))


if __name__ == "__main__":
    main()
