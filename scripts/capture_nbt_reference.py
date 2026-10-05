"""Capture native anonymous NBT bytes from the pinned Minecraft 26.1 NbtIo."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
JAR_HASH = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    source = ROOT / "scripts/NbtWireReference.java"
    decoder_path = ROOT / "scripts/jigsaw-reference/capture.py"
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    if hashlib.sha256(jar.read_bytes()).hexdigest() != JAR_HASH:
        raise ValueError("unexpected native JAR")
    if not args.build_dir.parent.is_dir() or not args.output.parent.is_dir():
        raise ValueError("capture parents must exist")
    args.build_dir.mkdir(exist_ok=True)
    subprocess.run(["javac", "-encoding", "UTF-8", "-d", str(args.build_dir), str(source)], check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [args.build_dir.resolve(), jar, *libraries]))
    run = subprocess.run([str(ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"),
                          "-Xmx512m", "-cp", classpath, "NbtWireReference"],
                         cwd=args.build_dir, capture_output=True, text=True, encoding="utf-8", timeout=120)
    (args.build_dir / "stdout.log").write_text(run.stdout, encoding="utf-8")
    (args.build_dir / "stderr.log").write_text(run.stderr, encoding="utf-8")
    if run.returncode:
        raise RuntimeError(run.stderr + run.stdout)
    marker = "NBT_WIRE_REFERENCE="
    line = next(line for line in run.stdout.splitlines() if marker in line)
    samples = json.loads(line.split(marker, 1)[1])
    spec = importlib.util.spec_from_file_location("jigsaw_nbt_reader", decoder_path)
    decoder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(decoder)
    for sample in samples:
        data = bytes.fromhex(sample["hex"])
        if sample["name"] == "float_nan_payload":
            sample["input_bits"] = "ff812345"
        elif sample["name"] == "double_nan_payload":
            sample["input_bits"] = "fff0000000000001"
        else:
            reader = decoder.NbtReader(data)
            kind = reader.number("B")
            sample["nbt"] = reader.payload(kind)
            if reader.stream.read(1):
                raise ValueError("native NBT was not fully decoded")
    result = {"minecraft": "26.1", "jar_sha256": JAR_HASH,
              "probe_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "capture_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "decoder_sha256": hashlib.sha256(decoder_path.read_bytes()).hexdigest(),
              "compound_order": "Java TreeMap; fixture compound keys have no ordering ambiguity",
              "samples": samples}
    if args.verify and json.loads(args.verify.read_text(encoding="utf-8")) != result:
        raise ValueError("native NBT fixture changed")
    args.output.write_text(json.dumps(result, ensure_ascii=True, allow_nan=False, indent=2) + "\n", encoding="utf-8")
    print(f"Captured {len(samples)} native anonymous NBT cases" + ("; verification passed" if args.verify else ""))


if __name__ == "__main__":
    main()
