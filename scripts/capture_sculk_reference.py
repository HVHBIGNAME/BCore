"""Capture the pinned 26.1 sculk oracle in its own Java build directory."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCES = (
    "TreeReference.java", "NativeEntityLevel.java", "NativeWorldgenRegistries.java",
    "TreeEffectReference.java", "SculkReference.java",
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java", type=Path, required=True)
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--vanilla", type=Path, default=ROOT / "target/vanilla-775")
    parser.add_argument("--build-dir", type=Path, default=ROOT / "target/sculk-reference")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "crates/bcore-worldgen/data")
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--data-only", action="store_true")
    parser.add_argument("--trace", action="store_true")
    args = parser.parse_args()
    jar = args.vanilla / "versions/26.1/server-26.1.jar"
    if not jar.is_file() or not args.build_dir.parent.is_dir() or not args.output_dir.is_dir():
        parser.error("the 26.1 JAR and build/output parent directories must exist")
    args.build_dir.mkdir(exist_ok=True)
    sources = [ROOT / "scripts" / source for source in SOURCES]
    subprocess.run([args.javac, "-d", str(args.build_dir), *map(str, sources)], check=True)
    classpath = os.pathsep.join(map(str, [args.build_dir, jar, *sorted((args.vanilla / "libraries").rglob("*.jar"))]))
    command = [str(args.java.resolve()), "-XX:ActiveProcessorCount=2", "-Xmx2G", "-cp", classpath, "SculkReference"]
    if args.data_only:
        command.append("--data-only")
    if args.trace:
        command.append("--trace")
    result = subprocess.run(command, cwd=args.build_dir, capture_output=True, text=True, timeout=600)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    marker = "SCULK_REFERENCE="
    records = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(records) != 1:
        raise RuntimeError("missing unique sculk oracle result: " + result.stdout + result.stderr)
    payload = json.loads(records[0])
    provenance = {
        "minecraft": "26.1", "jar_sha256": hashlib.sha256(jar.read_bytes()).hexdigest(),
        "probe_sha256": hashlib.sha256(b"".join(source.read_bytes() for source in sources)).hexdigest(),
        "source_dependencies": list(SOURCES),
    }
    outputs = {"sculk_states_26_1.json": {**provenance, **payload["data"]}}
    if not args.data_only:
        outputs["sculk_26_1.json"] = {**provenance, "setup": payload["setup"], "samples": payload["samples"], "kernels": payload["kernels"], "streams": payload["streams"]}
    for name, value in outputs.items():
        path = args.output_dir / name
        if args.verify:
            if json.loads(path.read_text(encoding="utf-8")) != value:
                raise ValueError(f"native capture differs from {path}")
        else:
            text = json.dumps(value, indent=2)
            text = re.sub(r"\[\n\s*(-?\d+(?:,\n\s*-?\d+)*)\n\s*\]",
                          lambda match: "[" + ", ".join(re.findall(r"-?\d+", match[1])) + "]", text)
            path.write_text(text + "\n", encoding="utf-8")
        print(f"{'Verified' if args.verify else 'Captured'} {path}")
    print(f"{payload['data']['state_count']} state predicates; {len(payload['samples'])} native feature cases; {len(payload['kernels'])} cursor cases; {len(payload['streams'])} complete placed streams")


if __name__ == "__main__":
    main()
