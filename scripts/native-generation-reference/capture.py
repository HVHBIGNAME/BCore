"""Capture real 26.1 ChunkMap execution in a new, never-reused server directory."""

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
STATUSES = (
    "empty", "structure_starts", "structure_references", "biomes", "noise", "surface",
    "carvers", "features", "initialize_light", "light", "spawn", "full",
)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def parse_request(text):
    x, z, status = text.split(",")
    if status not in STATUSES:
        raise argparse.ArgumentTypeError(f"unknown status {status!r}")
    return {"pos": [int(x), int(z)], "status": status}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=846692123413862008)
    parser.add_argument("--request", action="append", type=parse_request)
    parser.add_argument("--bootstrap", choices=("before_spawn", "native"), default="before_spawn")
    parser.add_argument("--processors", type=int, default=2)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--feature-snapshots", action="append", default=[], help="also snapshot a named placed feature's entire 3x3 write square")
    parser.add_argument("--timeout", type=int, default=900)
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / "target"):
        parser.error("capture output must be an isolated new directory under this repository's target/")
    if output.exists():
        parser.error(f"refusing to reuse any capture/world/build directory: {output}")
    vanilla = ROOT / "target/vanilla-775"
    jar = vanilla / "versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    if sha(jar) != JAR_SHA256:
        parser.error("pinned native JAR SHA-256 mismatch")
    if not -(1 << 63) <= args.seed < (1 << 63):
        parser.error("seed is outside signed i64")
    requests = args.request or [{"pos": [0, 0], "status": "features"}]
    watch = list(dict.fromkeys(tuple(r["pos"]) for r in requests))
    output.mkdir(parents=True)
    build = output / "build"
    server = output / "server"
    build.mkdir()
    server.mkdir()
    sources = [HERE / name for name in ("NativeAccess.java", "HistoryAgent.java", "HistoryProbe.java")]
    for source in sources:
        (build / source.name).write_bytes(source.read_bytes())
    (build / "capture.py").write_bytes(Path(__file__).read_bytes())
    asm_url = "https://repo.maven.apache.org/maven2/org/ow2/asm/asm/9.8/asm-9.8.jar"
    asm = build / "asm-9.8.jar"
    asm.write_bytes(urllib.request.urlopen(asm_url, timeout=60).read())
    expected_sha1 = urllib.request.urlopen(asm_url + ".sha1", timeout=60).read().decode().strip()
    if hashlib.sha1(asm.read_bytes()).hexdigest() != expected_sha1:
        raise RuntimeError("ASM download digest differs from Maven Central")
    compile_command = ["javac", "-cp", str(asm), "-d", str(build), *map(str, sources)]
    subprocess.run(compile_command, check=True)
    agent = build / "history-agent.jar"
    with zipfile.ZipFile(agent, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: HistoryAgent\n\n")
        for file in sorted(build.glob("*.class")):
            archive.write(file, file.name)
    config = {
        "schema": 5, "output": str(output), "minecraft": "26.1", "protocol": 775,
        "seed": str(args.seed), "bootstrap": args.bootstrap, "requests": requests,
        "watch": watch, "processors": args.processors, "workers": args.workers,
        "feature_snapshots": args.feature_snapshots,
    }
    config_path = output / "config.json"
    config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    properties = "\n".join((
        "level-name=oracle-world", f"level-seed={args.seed}", "level-type=minecraft:normal",
        "generate-structures=true", "server-ip=127.0.0.1", "server-port=0", "online-mode=false",
        "enable-rcon=false", "enable-query=false", "management-server-enabled=false",
        "sync-chunk-writes=false", "max-tick-time=-1", "view-distance=2", "simulation-distance=2",
        "pause-when-empty-seconds=0", "allow-nether=true", "enforce-secure-profile=false", "",
    ))
    (server / "server.properties").write_text(properties, encoding="utf-8")
    (server / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    libraries = sorted((vanilla / "libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [agent, asm, jar, *libraries]))
    command = [str(java), "-Xmx2G", f"-XX:ActiveProcessorCount={args.processors}",
               f"-Dmax.bg.threads={args.workers}",
               f"-javaagent:{agent}={config_path}", "-cp", classpath, "net.minecraft.server.Main", "--nogui"]
    provenance = {
        "minecraft": "26.1", "protocol": 775, "jar": str(jar), "jar_sha256": sha(jar),
        "sources": {str(p.relative_to(ROOT)): sha(p) for p in sources},
        "runner_sha256": sha(Path(__file__)), "agent_sha256": sha(agent),
        "asm_url": asm_url, "asm_sha256": sha(asm), "asm_sha1": expected_sha1,
        "config_sha256": sha(config_path), "server_properties": properties,
        "java": subprocess.run([str(java), "-version"], capture_output=True, text=True, check=True).stderr,
        "javac": subprocess.run(["javac", "-version"], capture_output=True, text=True, check=True).stdout,
        "compile_command": compile_command, "command": command, "cwd": str(server),
        "libraries": {str(p.relative_to(vanilla)): sha(p) for p in libraries},
        "bootstrap_contract": (
            "Stop at native MinecraftServer.setInitialSpawn entry; native ServerLevel and ChunkMap have been constructed. "
            "Do not run spawn search, prepareLevels or gameplay ticks. Record observed holder count."
            if args.bootstrap == "before_spawn" else
            "Run native loadLevel including spawn search and preparation, recording its actual requests; stop before gameplay ticks."
        ),
        "observation_barrier": "ChunkStep.apply futures complete only after the observer snapshots; native task bodies, queues and dependencies are not replaced.",
    }
    (output / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    with (output / "server.log").open("w", encoding="utf-8") as log:
        completed = subprocess.run(command, cwd=server, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
    if completed.returncode:
        raise RuntimeError(f"native capture failed ({completed.returncode}); preserve and inspect {output / 'server.log'}")
    events = [json.loads(line) for line in (output / "events.jsonl").read_text(encoding="utf-8").splitlines()]
    if not events or events[-1]["event"] != "oracle_complete":
        raise RuntimeError(f"native process did not reach the oracle terminal event: {output}")
    terminal = events[-1]
    summary = {"events": len(events), "feature_sources": terminal["feature_sources"],
               "inflight_stages": terminal["inflight_stages"],
               "gameplay_ticks": terminal["gameplay_ticks"], "events_sha256": sha(output / "events.jsonl"),
               "provenance_sha256": sha(output / "provenance.json")}
    (output / "capture.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(output), **summary}, indent=2))


if __name__ == "__main__":
    main()
