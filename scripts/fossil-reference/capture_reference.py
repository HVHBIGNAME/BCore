"""Capture all fossil rotations/templates and contextual rejection/tick cases."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from capture_assets import ROOT, PINNED, sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--bundle", action="store_true")
    args = parser.parse_args()
    out = args.output.resolve()
    if not out.is_relative_to(ROOT / "target") or out.exists():
        parser.error("choose a new immutable target directory")
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    assert sha(jar) == PINNED
    sources = [ROOT / "scripts/TreeReference.java", ROOT / "scripts/NativeWorldgenRegistries.java",
               ROOT / "scripts/jigsaw-reference/JigsawSupport.java", ROOT / "scripts/fossil-reference/FossilReference.java"]
    out.mkdir()
    for source in [*sources, Path(__file__)]:
        (out / source.name).write_bytes(source.read_bytes())
    compile_command = ["javac", "-d", str(out), *map(str, sources)]
    subprocess.run(compile_command, check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    command = [str(java), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-cp", os.pathsep.join(map(str, [out, jar, *libraries])), "FossilReference"]
    header = {"minecraft": "26.1", "jar_sha256": PINNED,
              "sources": {p.relative_to(ROOT).as_posix(): sha(p) for p in sources}, "capture_script_sha256": sha(Path(__file__))}
    (out / "provenance.json").write_text(json.dumps({**header, "command": command, "compile_command": compile_command}, indent=2) + "\n")
    result = subprocess.run(command, cwd=out, capture_output=True, text=True, timeout=600)
    (out / "stdout.log").write_text(result.stdout, encoding="utf-8")
    (out / "stderr.log").write_text(result.stderr, encoding="utf-8")
    result.check_returncode()
    markers = [line.partition("FOSSILREFERENCE=")[2] for line in result.stdout.splitlines() if "FOSSILREFERENCE=" in line]
    if len(markers) != 1:
        raise ValueError("incomplete native fossil component capture")
    data = {**header, **json.loads(markers[0])}
    encoded = (json.dumps(data, separators=(",", ":"), sort_keys=True) + "\n").encode()
    path = out / "fossil_reference_26_1.json"
    path.write_bytes(encoded)
    if args.bundle:
        with (ROOT / "crates/bcore-worldgen/data/fossil_reference_26_1.json").open("xb") as stream:
            stream.write(encoded)
    summary = {"cases": len(data["cases"]), "placed": sum(c["result"] for c in data["cases"]),
        "writes": sum(len(c["writes"]) for c in data["cases"]), "ticks": sum(len(c["ticks"]) for c in data["cases"]),
        "shape_order": data["shape_order"], "sha256": sha(path)}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
