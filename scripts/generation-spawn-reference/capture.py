"""Run genuine 26.1 generation-spawn methods in an isolated, immutable capture."""

import argparse
import hashlib
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
    parser.add_argument("--mode", choices=("catalog", "assets", "callbacks", "boundaries", "suite", "entry", "handoff", "inputs"), default="suite")
    parser.add_argument("--seed", type=int, default=846692123413862008)
    parser.add_argument("--timeout", type=int, default=900)
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / "target") or output.exists():
        parser.error("output must be a new, never-reused directory inside the repository's target/")
    vanilla = ROOT / "target/vanilla-775"
    jar = vanilla / "versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    if sha(jar) != JAR_SHA256:
        parser.error("pinned native server digest mismatch")
    output.mkdir(parents=True)
    build = output / "build"
    server = output / "server"
    build.mkdir()
    server.mkdir()
    sources = [HERE / "GenerationSpawnAgent.java", HERE / "GenerationSpawnProbe.java", HERE / "GenerationSpawnHandoff.java",
               HERE / "GenerationSpawnInputs.java",
               ROOT / "scripts/native-generation-reference/NativeAccess.java"]
    for source in sources:
        (build / source.name).write_bytes(source.read_bytes())
    (build / "capture.py").write_bytes(Path(__file__).read_bytes())
    url = "https://repo.maven.apache.org/maven2/org/ow2/asm/asm/9.8/asm-9.8.jar"
    asm = build / "asm-9.8.jar"
    asm.write_bytes(urllib.request.urlopen(url, timeout=60).read())
    expected = urllib.request.urlopen(url + ".sha1", timeout=60).read().decode().strip()
    if hashlib.sha1(asm.read_bytes()).hexdigest() != expected:
        raise RuntimeError("ASM Maven digest mismatch")
    compile_command = ["javac", "-cp", str(asm), "-d", str(build), *map(str, sources)]
    subprocess.run(compile_command, check=True)
    agent = build / "generation-spawn-agent.jar"
    with zipfile.ZipFile(agent, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: GenerationSpawnAgent\n\n")
        for file in sorted(build.glob("*.class")):
            archive.write(file, file.name)
    config = {"schema": 1, "minecraft": "26.1", "protocol": 775, "output": str(output),
              "mode": args.mode, "seed": str(args.seed)}
    config_path = output / "config.json"
    config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    properties = "\n".join((
        "level-name=spawn-oracle", f"level-seed={args.seed}", "level-type=minecraft:normal",
        "generate-structures=true", "server-ip=127.0.0.1", "server-port=0", "online-mode=false",
        "enable-rcon=false", "enable-query=false", "management-server-enabled=false",
        "sync-chunk-writes=false", "max-tick-time=-1", "view-distance=2", "simulation-distance=2",
        "pause-when-empty-seconds=0", "enforce-secure-profile=false", "",
    ))
    (server / "server.properties").write_text(properties, encoding="utf-8")
    (server / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    libraries = sorted((vanilla / "libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [agent, asm, jar, *libraries]))
    command = [str(java), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
               f"-javaagent:{agent}={config_path}", "-cp", classpath,
               "net.minecraft.server.Main", "--nogui"]
    provenance = {"jar_sha256": sha(jar), "sources": {str(p.relative_to(ROOT)): sha(p) for p in sources},
                  "runner_sha256": sha(Path(__file__)), "asm_sha256": sha(asm),
                  "java": subprocess.run([str(java), "-version"], capture_output=True, text=True).stderr,
                  "compile_command": compile_command, "command": command, "cwd": str(server),
                  "entropy_contract": "Entity RandomSource.create at native Entity constructor is supplied independent explicit test seeds. Native UUID draws and native constructors/finalizers execute unchanged. No world-seeded UUID claim.",
                  "bootstrap": "Native server initialization, intercepted at setInitialSpawn before spawn search or gameplay ticks."}
    (output / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    with (output / "server.log").open("x", encoding="utf-8") as log:
        completed = subprocess.run(command, cwd=server, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
    if completed.returncode:
        raise RuntimeError(f"native capture failed ({completed.returncode}); inspect {output / 'server.log'}")
    fixture = output / "generation-spawn.json"
    if not fixture.exists():
        raise RuntimeError("native fixture not produced")
    print(json.dumps({"output": str(output), "fixture_sha256": sha(fixture),
                      "provenance_sha256": sha(output / "provenance.json")}, indent=2))


if __name__ == "__main__":
    main()
