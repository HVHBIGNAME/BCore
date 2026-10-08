"""Capture native fossil templates and provenance in a new immutable directory."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
PINNED = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    out = args.output.resolve()
    if not out.is_relative_to(ROOT / "target") or out.exists():
        parser.error("choose a new directory under this worktree's target")
    jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    assert sha(jar) == PINNED
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    sources = [ROOT / "scripts/TreeReference.java", ROOT / "scripts/NativeWorldgenRegistries.java",
               ROOT / "scripts/jigsaw-reference/JigsawSupport.java", ROOT / "scripts/fossil-reference/FossilAssets.java"]
    out.mkdir(parents=True)
    for source in [*sources, Path(__file__)]:
        (out / source.name).write_bytes(source.read_bytes())
    compile_command = ["javac", "-d", str(out), *map(str, sources)]
    subprocess.run(compile_command, check=True)
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    command = [str(java), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-cp", os.pathsep.join(map(str, [out, jar, *libraries])), "FossilAssets"]
    header = {"minecraft": "26.1", "jar_sha256": PINNED,
              "sources": {p.relative_to(ROOT).as_posix(): sha(p) for p in sources},
              "capture_script_sha256": sha(Path(__file__))}
    (out / "provenance.json").write_text(json.dumps({**header, "compile_command": compile_command, "command": command}, indent=2) + "\n")
    result = subprocess.run(command, cwd=out, capture_output=True, text=True, timeout=600)
    (out / "stdout.log").write_text(result.stdout, encoding="utf-8")
    (out / "stderr.log").write_text(result.stderr, encoding="utf-8")
    result.check_returncode()
    markers = [line.partition("FOSSILASSETS=")[2] for line in result.stdout.splitlines() if "FOSSILASSETS=" in line]
    if len(markers) != 1:
        raise ValueError("incomplete native fossil asset capture")
    data = {**header, **json.loads(markers[0])}
    with zipfile.ZipFile(jar) as archive:
        data["template_sources"] = {name: hashlib.sha256(archive.read("data/minecraft/structure/" + name.removeprefix("minecraft:") + ".nbt")).hexdigest()
                                    for name in data["templates"]}
        data["processor_sources"] = {}
        for name in ("fossil_rot", "fossil_coal", "fossil_diamonds"):
            raw = archive.read("data/minecraft/worldgen/processor_list/" + name + ".json")
            data["processor_sources"][name] = {"sha256": hashlib.sha256(raw).hexdigest(), "value": json.loads(raw)}
    path = out / "fossil_assets_26_1.json"
    path.write_text(json.dumps(data, separators=(",", ":"), sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(path), "sha256": sha(path), "templates": len(data["templates"]),
                      "blocks": sum(len(p) for t in data["templates"].values() for p in t["palettes"])}, indent=2))


if __name__ == "__main__":
    main()
