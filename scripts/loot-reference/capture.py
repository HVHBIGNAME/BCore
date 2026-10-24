"""Run the bounded loot oracle through the repository's serialized heavy-check queue."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import zipfile

import inventory
from verify import verify_capture

ROOT = inventory.ROOT
HERE = Path(__file__).resolve().parent
JAVA = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
ASM = ROOT / "target/native-environment-restoration-20261014-01/asm/asm-9.8.jar"
ASM_SHA256 = "876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051"


def sha(path):
    return inventory.digest(path.read_bytes())


def check_input(spec):
    if spec.get("schema") != 1 or not 1 <= len(spec.get("cases", [])) <= 256:
        raise ValueError("oracle requires schema 1 and 1..256 explicit cases")
    if spec["context"]["luck"] != 0 or spec["context"]["player"] is not None:
        raise ValueError("this oracle version captures luck 0 with no player only")
    ids = set()
    for case in spec["cases"]:
        seed = int(case["seed"])
        if not -(1 << 63) <= seed < (1 << 63) or seed == 0:
            raise ValueError("this oracle version requires nonzero signed-i64 loot seeds")
        if not isinstance(case["size"], int) or not 0 <= case["size"] <= 54:
            raise ValueError("container size outside 0..54")
        if case["id"] in ids:
            raise ValueError("duplicate case ID")
        ids.add(case["id"])
        slots = [entry["slot"] for entry in case["preexisting"]]
        if len(slots) != len(set(slots)) or any(not 0 <= slot < case["size"] for slot in slots):
            raise ValueError("invalid preexisting slot")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cases", type=Path, default=HERE / "cases-initial.json")
    parser.add_argument("--repeat-of", type=Path)
    parser.add_argument("--timeout", type=int, default=900)
    args = parser.parse_args()
    if os.environ.get("CARGO_TARGET_DIR") != str(ROOT / "target/build") or os.environ.get("CARGO_BUILD_JOBS") != "1":
        parser.error("launch only through root scripts/run_generation_check.py --background label -- command")
    if sha(inventory.JAR) != inventory.JAR_SHA256 or sha(ASM) != ASM_SHA256:
        parser.error("pinned server JAR or ASM digest mismatch")
    javac = shutil.which("javac")
    if javac is None:
        parser.error("javac21 must be available on PATH")
    spec_bytes = args.cases.read_bytes()
    spec = json.loads(spec_bytes)
    check_input(spec)
    out = inventory.new_output(args.output)
    build, server_dir = out / "build", out / "server"
    build.mkdir()
    server_dir.mkdir()
    inventory.export(out)
    sources = [HERE / "LootAgent.java", HERE / "LootProbe.java", ROOT / "scripts/native-generation-reference/NativeAccess.java"]
    frozen = [*sources, HERE / "capture.py", HERE / "inventory.py", HERE / "verify.py"]
    for source in frozen:
        with (build / source.name).open("xb") as stream:
            stream.write(source.read_bytes())
    with (build / "cases.json").open("xb") as stream:
        stream.write(spec_bytes)
    with (build / ASM.name).open("xb") as stream:
        stream.write(ASM.read_bytes())
    config = {**spec, "output": str(out)}
    inventory.write_json(out / "config.json", config)
    sources_sha = {str(p.relative_to(ROOT)): sha(build / p.name) for p in frozen}
    if args.repeat_of:
        previous = json.loads((args.repeat_of / "provenance.json").read_text(encoding="utf-8"))
        if sources_sha != previous["sources"] or inventory.digest(spec_bytes) != previous["cases_sha256"]:
            raise ValueError("independent repeat requires identical frozen sources and case input")
    compile_command = [javac, "--release", "21", "-cp", str(build / ASM.name), "-d", str(build),
                       *[str(build / source.name) for source in sources]]
    with (out / "compile.log").open("x", encoding="utf-8") as log:
        subprocess.run(compile_command, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=120)
    agent = build / "loot-agent.jar"
    with zipfile.ZipFile(agent, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: LootAgent\n\n")
        for path in sorted(build.glob("*.class")):
            archive.write(path, path.name)
    properties = "\n".join((
        "level-name=loot-oracle-world", f"level-seed={spec['world_seed']}", "level-type=minecraft:normal",
        "generate-structures=true", "server-ip=127.0.0.1", "server-port=0", "online-mode=false",
        "enable-rcon=false", "enable-query=false", "management-server-enabled=false", "sync-chunk-writes=false",
        "max-tick-time=-1", "view-distance=2", "simulation-distance=2", "pause-when-empty-seconds=0",
        "enforce-secure-profile=false", ""))
    (server_dir / "server.properties").write_text(properties, encoding="utf-8")
    (server_dir / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [agent, build / ASM.name, inventory.JAR, *libraries]))
    command = [str(JAVA), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
               f"-javaagent:{agent}={out / 'config.json'}", "-cp", classpath, "net.minecraft.server.Main", "--nogui"]
    provenance = {"schema": 1, "minecraft": "26.1", "protocol": 775, "jar": str(inventory.JAR),
        "jar_sha256": sha(inventory.JAR), "asm_sha256": sha(build / ASM.name), "java_executable_sha256": sha(JAVA),
        "javac_executable_sha256": sha(Path(javac)), "sources": sources_sha,
        "cases_source": str(args.cases.resolve()), "cases_sha256": inventory.digest(spec_bytes),
        "agent_sha256": sha(agent), "compiled_classes": {p.name: sha(p) for p in sorted(build.glob("*.class"))},
        "config_sha256": sha(out / "config.json"), "compile_command": compile_command, "command": command,
        "java": subprocess.run([str(JAVA), "-version"], capture_output=True, text=True, check=True, timeout=30).stderr,
        "javac": subprocess.run([javac, "-version"], capture_output=True, text=True, check=True, timeout=30).stdout,
        "libraries": {str(p.relative_to(ROOT)): sha(p) for p in libraries}, "server_properties": properties,
        "queue_environment": {k: os.environ.get(k) for k in ("CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "RAYON_NUM_THREADS")},
        "authority": "Actual ServerLevel, native loaded datapack registries, unmodified loot method bodies plus observation hooks.",
        "bootstrap": "MinecraftServer.setInitialSpawn entry. Native FULL chunk and chest placement; zero gameplay ticks.",
        "limitations": ["No player, luck=0 only", "Seed0 and random-sequence continuation not captured by this version",
                        "No performance measurements: observation hooks deliberately add overhead",
                        "Native fill/unpack tests are not a complete inventory of world-dependent loot function behavior"]}
    inventory.write_json(out / "provenance.json", provenance)
    with (out / "server.log").open("x", encoding="utf-8") as log:
        completed = subprocess.run(command, cwd=server_dir, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
    if completed.returncode != 0 or not (out / "observations.raw.json").exists():
        raise RuntimeError(f"native oracle failed with {completed.returncode}; preserve {out / 'server.log'}")
    if sha(inventory.JAR) != inventory.JAR_SHA256:
        raise RuntimeError("pinned JAR changed during capture")
    verification = verify_capture(out, args.repeat_of)
    manifest = {p.relative_to(out).as_posix(): sha(p) for p in sorted(out.rglob("*")) if p.is_file()}
    inventory.write_json(out / "manifest.json", manifest)
    print(json.dumps({"output": str(out), "status": verification["status"], "cases": verification["case_count"],
                      "native_fills": verification["native_fills"], "rng_checks": verification["verified_rng_operations"],
                      "independent_repeat_matches": bool(verification["independent_repeat"]),
                      "fixtures_sha256": verification["fixtures_sha256"]}, indent=2))


if __name__ == "__main__":
    main()
