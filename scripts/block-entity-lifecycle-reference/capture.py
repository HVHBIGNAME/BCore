"""Immutable native BE lifecycle captures on a real pre-spawn ServerLevel."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeat-of", type=Path)
    args = parser.parse_args()
    out = args.output.resolve()
    if not out.is_relative_to(ROOT / "target") or out.exists():
        parser.error("output must be a never-used directory under this worktree's target/")
    vanilla = ROOT / "target/vanilla-775"
    jar = vanilla / "versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    if sha(jar) != JAR_SHA256:
        parser.error("wrong native JAR digest")
    out.mkdir(parents=True)
    build, server = out / "build", out / "server"
    build.mkdir()
    server.mkdir()
    inputs = [HERE / "LifecycleAgent.java", HERE / "LifecycleProbe.java",
              ROOT / "scripts/native-generation-reference/NativeAccess.java"]
    for source in [*inputs, Path(__file__)]:
        (build / source.name).write_bytes(source.read_bytes())
    asm = build / "asm-9.8.jar"
    url = "https://repo.maven.apache.org/maven2/org/ow2/asm/asm/9.8/asm-9.8.jar"
    asm.write_bytes(urllib.request.urlopen(url, timeout=60).read())
    expected_sha1 = urllib.request.urlopen(url + ".sha1", timeout=60).read().decode().strip()
    if hashlib.sha1(asm.read_bytes()).hexdigest() != expected_sha1:
        raise RuntimeError("ASM download digest mismatch")
    command = ["javac", "-cp", str(asm), "-d", str(build), *map(str, inputs)]
    subprocess.run(command, check=True)
    agent = build / "lifecycle-agent.jar"
    with zipfile.ZipFile(agent, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: LifecycleAgent\n\n")
        for file in sorted(build.glob("*.class")):
            archive.write(file, file.name)
    properties = "\n".join((
        "level-name=lifecycle-world", "level-seed=846692123413862008", "level-type=minecraft:normal",
        "generate-structures=true", "server-ip=127.0.0.1", "server-port=0", "online-mode=false",
        "enable-rcon=false", "enable-query=false", "management-server-enabled=false",
        "sync-chunk-writes=false", "max-tick-time=-1", "view-distance=2", "simulation-distance=2",
        "pause-when-empty-seconds=0", "enforce-secure-profile=false", ""))
    (server / "server.properties").write_text(properties, encoding="utf-8")
    (server / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    libraries = sorted((vanilla / "libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [agent, asm, jar, *libraries]))
    run = [str(java), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
           f"-javaagent:{agent}={out}", "-cp", classpath, "net.minecraft.server.Main", "--nogui"]
    provenance = {"jar_sha256": sha(jar), "source_sha256": {str(p.relative_to(ROOT)): sha(p) for p in inputs},
                  "runner_sha256": sha(Path(__file__)), "compile_command": command, "command": run,
                  "server_properties": properties, "asm_sha256": sha(asm),
                  "java": subprocess.run([str(java), "-version"], capture_output=True, text=True, check=True).stderr,
                  "libraries": {str(p.relative_to(vanilla)): sha(p) for p in libraries}}
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    for name in ["ProtoChunk", "ChunkAccess", "LevelChunk"]:
        result = subprocess.run(["javap", "-c", "-p", "-classpath", str(jar), "net.minecraft.world.level.chunk." + name],
                                check=True, capture_output=True, text=True)
        (out / (name + ".javap.txt")).write_text(result.stdout, encoding="utf-8")
    result = subprocess.run(["javap", "-c", "-p", "-classpath", str(jar), "net.minecraft.server.level.WorldGenRegion"],
                            check=True, capture_output=True, text=True)
    (out / "WorldGenRegion.javap.txt").write_text(result.stdout, encoding="utf-8")
    with (out / "server.log").open("x", encoding="utf-8") as log:
        completed = subprocess.run(run, cwd=server, stdout=log, stderr=subprocess.STDOUT, timeout=600)
    if completed.returncode:
        raise RuntimeError(f"native lifecycle failed: {out / 'server.log'}")
    log = (out / "server.log").read_text(encoding="utf-8")
    if "LIFECYCLE_COMPLETE" not in log or "Serialization errors:" in log:
        raise RuntimeError("incomplete native capture or serialization failure")
    spec = importlib.util.spec_from_file_location("jigsaw_capture", ROOT / "scripts/jigsaw-reference/capture.py")
    codec = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(codec)
    raw = json.loads((out / "observations.raw.json").read_text(encoding="utf-8"))
    data = {"jar_sha256": JAR_SHA256, **codec.decode_fixture(raw)}
    (out / "observations.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    if args.repeat_of:
        prior = json.loads((args.repeat_of / "observations.json").read_text(encoding="utf-8"))
        if data != prior:
            raise RuntimeError("independent native lifecycle recapture differs")
    manifest = {p.name: sha(p) for p in sorted(out.glob("*")) if p.is_file()}
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(out), "cases": len(data["cases"]),
                      "observations_sha256": sha(out / "observations.json"),
                      "independent_repeat_matches": bool(args.repeat_of)}, indent=2))


if __name__ == "__main__":
    main()
