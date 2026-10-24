"""Record pinned server bytecode for the implicit-FULL scheduler investigation.

Run through scripts/run_generation_check.py --background. This disassembles
class files; it does not initialize Minecraft classes or launch the server.
"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import zipfile


ROOT = Path(__file__).resolve().parents[2]
JAR = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
CLASSES = (
    "net.minecraft.server.level.ServerChunkCache",
    "net.minecraft.server.level.ServerChunkCache$MainThreadExecutor",
    "net.minecraft.server.level.ChunkLevel",
    "net.minecraft.server.level.DistanceManager",
    "net.minecraft.server.level.LoadingChunkTracker",
    "net.minecraft.world.level.TicketStorage",
    "net.minecraft.server.level.TicketType",
    "net.minecraft.server.level.ChunkMap",
    "net.minecraft.server.level.ChunkHolder",
    "net.minecraft.server.level.GenerationChunkHolder",
    "net.minecraft.server.level.ChunkGenerationTask",
    "net.minecraft.server.level.ChunkTaskDispatcher",
    "net.minecraft.world.level.chunk.status.ChunkStatusTasks",
    "net.minecraft.world.level.chunk.status.ChunkDependencies",
    "net.minecraft.util.thread.BlockableEventLoop",
    "net.minecraft.util.thread.ReentrantBlockableEventLoop",
)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--class", dest="classes", action="append")
    parser.add_argument("--verbose", action="store_true", help="include bootstrap method handles")
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / "target") or output.exists():
        parser.error("output must be a new directory under target/")
    jar_bytes = JAR.read_bytes()
    if sha(jar_bytes) != JAR_SHA256:
        parser.error("pinned server JAR digest mismatch")
    executable = shutil.which("javap")
    if executable is None:
        parser.error("javap must be available on PATH")
    output.mkdir(parents=True)
    source = Path(__file__).read_bytes()
    (output / "javap.py").write_bytes(source)
    version = subprocess.run(
        [executable, "-version"], capture_output=True, check=True, timeout=30
    )
    record = {
        "schema": 1,
        "jar": str(JAR),
        "jar_sha256": JAR_SHA256,
        "runner_sha256": sha(source),
        "javap": executable,
        "javap_sha256": sha(Path(executable).read_bytes()),
        "javap_version": version.stdout.decode("utf-8", errors="replace").strip(),
        "classes": [],
        "missing_classes": [],
    }
    with zipfile.ZipFile(JAR) as archive:
        members = set(archive.namelist())
        for name in args.classes or CLASSES:
            member = name.replace(".", "/") + ".class"
            if member not in members:
                record["missing_classes"].append(name)
                continue
            data = archive.read(member)
            command = [executable, "-classpath", str(JAR), "-c", "-p", "-s", "-constants", name]
            if args.verbose:
                command.insert(-1, "-v")
            result = subprocess.run(command, capture_output=True, timeout=90)
            filename = name + ".javap.txt"
            (output / filename).write_bytes(result.stdout)
            (output / (name + ".stderr.txt")).write_bytes(result.stderr)
            record["classes"].append({
                "name": name,
                "member": member,
                "class_sha256": sha(data),
                "class_major_version": int.from_bytes(data[6:8], "big"),
                "command": command,
                "exit_code": result.returncode,
                "output": filename,
                "output_sha256": sha(result.stdout),
                "stderr_sha256": sha(result.stderr),
            })
            (output / "provenance.json").write_text(
                json.dumps(record, indent=2) + "\n", encoding="utf-8"
            )
            result.check_returncode()
    if sha(JAR.read_bytes()) != JAR_SHA256:
        raise RuntimeError("server JAR changed during disassembly")
    (output / "provenance.json").write_text(
        json.dumps(record, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps({"output": str(output), "classes": len(record["classes"]),
                      "missing_classes": record["missing_classes"]}))


if __name__ == "__main__":
    main()
