"""Capture the original native FULL task independently of generation and ticking."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
ASM_SHA256 = "876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--asm", type=Path, required=True)
    parser.add_argument("--repeat-of", type=Path)
    args = parser.parse_args()
    out, asm = args.output.resolve(), args.asm.resolve()
    if not out.is_relative_to(ROOT / "target") or out.exists():
        parser.error("output must be a never-used directory under this worktree's target")
    vanilla = ROOT / "target/vanilla-775"
    jar = vanilla / "versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    if sha(jar) != JAR_SHA256 or sha(asm) != ASM_SHA256:
        parser.error("wrong pinned JAR or ASM digest")
    out.mkdir(parents=True)
    build, server = out / "build", out / "server"
    build.mkdir()
    server.mkdir()
    sources = [ROOT / "scripts/block-entity-lifecycle-reference/LifecycleAgent.java", HERE / "LifecycleProbe.java",
               ROOT / "scripts/native-generation-reference/NativeAccess.java"]
    for source in [*sources, Path(__file__)]:
        (build / source.name).write_bytes(source.read_bytes())
    (build / asm.name).write_bytes(asm.read_bytes())
    asm = build / asm.name
    compile_command = ["javac", "-cp", str(asm), "-d", str(build), *map(str, sources)]
    subprocess.run(compile_command, check=True)
    agent = build / "full-lifecycle-agent.jar"
    with zipfile.ZipFile(agent, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: LifecycleAgent\n\n")
        for path in sorted(build.glob("*.class")):
            archive.write(path, path.name)
    properties = "\n".join(("level-name=full-lifecycle-world", "level-seed=846692123413862008",
        "level-type=minecraft:normal", "generate-structures=true", "server-ip=127.0.0.1", "server-port=0",
        "online-mode=false", "enable-rcon=false", "enable-query=false", "management-server-enabled=false",
        "sync-chunk-writes=false", "max-tick-time=-1", "view-distance=2", "simulation-distance=2",
        "pause-when-empty-seconds=0", "enforce-secure-profile=false", ""))
    (server / "server.properties").write_text(properties, encoding="utf-8")
    (server / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    libraries = sorted((vanilla / "libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [agent, asm, jar, *libraries]))
    command = [str(java), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
               f"-javaagent:{agent}={out}", "-cp", classpath, "net.minecraft.server.Main", "--nogui"]
    provenance = {"jar_sha256": sha(jar), "asm_sha256": sha(asm), "agent_sha256": sha(agent),
        "sources": {str(p.relative_to(ROOT)): sha(p) for p in sources}, "runner_sha256": sha(Path(__file__)),
        "compile_command": compile_command, "command": command, "server_properties": properties,
        "java": subprocess.run([str(java), "-version"], capture_output=True, text=True, check=True).stderr,
        "javac": subprocess.run(["javac", "-version"], capture_output=True, text=True, check=True).stdout,
        "libraries": {str(p.relative_to(vanilla)): sha(p) for p in libraries}}
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    with (out / "server.log").open("x", encoding="utf-8") as log:
        result = subprocess.run(command, cwd=server, stdout=log, stderr=subprocess.STDOUT, timeout=300)
    log = (out / "server.log").read_text(encoding="utf-8")
    if result.returncode or "FULL_LIFECYCLE_COMPLETE" not in log or "Serialization errors:" in log:
        raise RuntimeError(f"native FULL capture failed: {out / 'server.log'}")
    spec = importlib.util.spec_from_file_location("codec", ROOT / "scripts/jigsaw-reference/capture.py")
    codec = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(codec)
    raw = json.loads((out / "observations.raw.json").read_text(encoding="utf-8"))
    data = {"jar_sha256": JAR_SHA256, **codec.decode_fixture(raw)}
    (out / "observations.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    if args.repeat_of:
        prior = json.loads((args.repeat_of / "observations.json").read_text(encoding="utf-8"))
        if data != prior:
            raise RuntimeError("independent native repeat differs")
    manifest = {p.name: sha(p) for p in sorted(out.iterdir()) if p.is_file()}
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(out), "observations_sha256": sha(out / "observations.json"),
                      "independent_repeat_matches": bool(args.repeat_of)}, indent=2))


if __name__ == "__main__":
    main()
