"""Run the unchanged native history observer plus a private admission companion.

Invoke only through scripts/run_generation_check.py --background. Settings and
baseline Java come from a selected existing capture; the new world is independent.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile


ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
ASM_SHA256 = "876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051"
BASELINE_SOURCES = {
    "HistoryAgent.java": "f36e81ff13bba0b7dd5069d602a724d0ce2839b8bcc90e8c74dc961d182efecc",
    "HistoryProbe.java": "1dba9a48dfb09831da580b4ccaa3c34c9395a5162f16b793839e306a63953013",
    "NativeAccess.java": "93f32d9a715ea0b82abfe262c5362efc0ad08c25cfa3c028d3c5bc792c5fd55f",
}
COMPANION_SOURCES = ("AdmissionAgent.java", "AdmissionObjects.java", "AdmissionProbe.java")


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def freeze_sources(reference, build):
    baseline = []
    for name, expected in BASELINE_SOURCES.items():
        source = reference / "build" / name
        data = source.read_bytes()
        if hashlib.sha256(data).hexdigest() != expected:
            raise ValueError(f"baseline Java differs from approved frozen bytes: {source}")
        frozen = build / name
        frozen.write_bytes(data)
        baseline.append(frozen)
    companion = []
    for name in COMPANION_SOURCES:
        frozen = build / name
        frozen.write_bytes((HERE / name).read_bytes())
        companion.append(frozen)
    (build / "capture.py").write_bytes(Path(__file__).read_bytes())
    (build / "baseline-capture.py").write_bytes((reference / "build/capture.py").read_bytes())
    (build / "reference-provenance.json").write_bytes((reference / "provenance.json").read_bytes())
    return baseline, companion


def compile_agent(javac, sources, classes, classpath, log):
    classes.mkdir()
    command = [javac, "-cp", classpath, "-d", str(classes), *map(str, sources)]
    with log.open("xb") as stream:
        result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT, timeout=120)
    result.check_returncode()
    return command


def agent_jar(path, classes, main):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", f"Manifest-Version: 1.0\nPremain-Class: {main}\n\n")
        for file in sorted(classes.glob("*.class")):
            archive.write(file, file.name)


def terminal_events(path):
    count, last = 0, None
    with path.open(encoding="utf-8") as stream:
        for count, line in enumerate(stream, 1):
            last = line
    return count, json.loads(last) if last is not None else None


def run(args, output):
    reference = args.reference.resolve()
    reference_provenance = load(reference / "provenance.json")
    reference_receipt = load(reference / "capture.json")
    if sha(reference / "provenance.json") != reference_receipt["provenance_sha256"]:
        raise ValueError("reference provenance does not match its capture receipt")
    if sha(reference / "config.json") != reference_provenance["config_sha256"]:
        raise ValueError("reference config digest mismatch")
    if reference_provenance["jar_sha256"] != JAR_SHA256:
        raise ValueError("reference is not the pinned native server")
    config = load(reference / "config.json")
    if config["schema"] != 5 or config["bootstrap"] != "before_spawn":
        raise ValueError("this private runner requires the approved schema-5 before-spawn observer")
    config["output"] = str(output)
    config_path = output / "config.json"
    save(config_path, config)
    vanilla = ROOT / "target/vanilla-775"
    jar = vanilla / "versions/26.1/server-26.1.jar"
    java = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
    if sha(jar) != JAR_SHA256 or sha(args.asm.resolve()) != ASM_SHA256:
        raise ValueError("pinned server or ASM digest mismatch")
    javac = shutil.which("javac")
    if javac is None:
        raise ValueError("javac is not on PATH")
    build, server = output / "build", output / "server"
    build.mkdir()
    server.mkdir()
    baseline, companion = freeze_sources(reference, build)
    asm = build / "asm-9.8.jar"
    shutil.copyfile(args.asm.resolve(), asm)
    libraries = sorted((vanilla / "libraries").rglob("*.jar"))
    library_hashes = {str(path.relative_to(vanilla)): sha(path) for path in libraries}
    if library_hashes != reference_provenance["libraries"]:
        raise ValueError("native libraries differ from the selected baseline capture")
    provenance = {
        "schema": 1, "minecraft": "26.1", "protocol": 775,
        "reference": str(reference), "reference_provenance_sha256": sha(reference / "provenance.json"),
        "reference_config_sha256": sha(reference / "config.json"),
        "jar": str(jar), "jar_sha256": JAR_SHA256, "libraries": library_hashes,
        "java_executable_sha256": sha(java), "javac_executable": javac, "javac_executable_sha256": sha(Path(javac)),
        "sources": {str(Path("scripts/native-generation-reference") / source.name): sha(source) for source in baseline},
        "companion_sources": {str(Path("scripts/implicit-full-reference") / source.name): sha(source) for source in companion},
        "runner": str(Path(__file__).relative_to(ROOT)), "runner_sha256": sha(build / "capture.py"),
        "frozen_baseline_runner_sha256": sha(build / "baseline-capture.py"),
        "asm_source": str(args.asm.resolve()), "asm_sha256": sha(asm),
        "config_sha256": sha(config_path), "server_properties": reference_provenance["server_properties"],
        "bootstrap_contract": reference_provenance["bootstrap_contract"],
        "observation_barrier": reference_provenance["observation_barrier"],
        "companion_contract": {
            "schema": 1, "events": "admission-events.jsonl", "identity": "independent ==-based IDs",
            "baseline_seq": "sampled while holding HistoryProbe.class, the baseline emission monitor",
            "future_wrapping": False, "additional_completion_barriers": False,
            "native_runnable_wrapping": False, "additional_serialization": False,
            "extra_boolean_supplier_calls": False, "drain_at_end": False,
            "baseline_observer_calls": "included and labeled baseline_observing",
            "sidecar_termination": "flushed event prefix sealed after successful baseline oracle_complete and process exit",
        },
        "commands": [],
    }
    provenance_path = output / "provenance.json"
    save(provenance_path, provenance)
    base_classes, companion_classes = build / "baseline-classes", build / "companion-classes"
    provenance["commands"].append(compile_agent(javac, baseline, base_classes, str(asm), build / "baseline-javac.log"))
    save(provenance_path, provenance)
    provenance["commands"].append(compile_agent(javac, companion, companion_classes,
        os.pathsep.join(map(str, (asm, base_classes))), build / "companion-javac.log"))
    history_agent, companion_agent = build / "history-agent.jar", build / "admission-agent.jar"
    agent_jar(history_agent, base_classes, "HistoryAgent")
    agent_jar(companion_agent, companion_classes, "AdmissionAgent")
    provenance["agent_sha256"] = sha(history_agent)
    provenance["companion_agent_sha256"] = sha(companion_agent)
    provenance["class_sha256"] = {str(path.relative_to(build)): sha(path)
        for classes in (base_classes, companion_classes) for path in sorted(classes.glob("*.class"))}
    for label, executable in (("java", str(java)), ("javac", javac)):
        command = [executable, "-version"]
        result = subprocess.run(command, capture_output=True, timeout=30, check=True)
        provenance[label] = (result.stderr if label == "java" else result.stdout).decode("utf-8", errors="replace")
        provenance["commands"].append(command)
    (server / "server.properties").write_text(provenance["server_properties"], encoding="utf-8")
    (server / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    classpath = os.pathsep.join(map(str, (history_agent, companion_agent, asm, jar, *libraries)))
    command = [str(java), "-Xmx2G", f"-XX:ActiveProcessorCount={config['processors']}",
        f"-Dmax.bg.threads={config['workers']}", f"-javaagent:{history_agent}={config_path}",
        f"-javaagent:{companion_agent}", "-cp", classpath, "net.minecraft.server.Main", "--nogui"]
    provenance["command"], provenance["cwd"] = command, str(server)
    provenance["commands"].append(command)
    save(provenance_path, provenance)
    with (output / "server.log").open("xb") as log:
        result = subprocess.run(command, cwd=server, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
    result.check_returncode()
    count, terminal = terminal_events(output / "events.jsonl")
    if terminal is None or terminal["event"] != "oracle_complete" or terminal["seq"] != count:
        raise ValueError("native run lacks a successful baseline terminal event")
    if sha(jar) != JAR_SHA256:
        raise ValueError("native JAR changed during capture")
    sidecar_count, sidecar_last = terminal_events(output / "admission-events.jsonl")
    if sidecar_last is None or sidecar_last["seq"] != sidecar_count or sidecar_last["baseline_seq"] > count:
        raise ValueError("invalid sidecar terminal prefix")
    summary = {"events": count, "feature_sources": terminal["feature_sources"],
        "inflight_stages": terminal["inflight_stages"], "gameplay_ticks": terminal["gameplay_ticks"],
        "events_sha256": sha(output / "events.jsonl"), "provenance_sha256": sha(provenance_path)}
    save(output / "capture.json", summary)
    sidecar = {"schema": 1, "events": sidecar_count, "events_sha256": sha(output / "admission-events.jsonl"),
        "baseline_terminal_seq": count, "last_sampled_baseline_seq": sidecar_last["baseline_seq"],
        "native_exit_code": result.returncode, "baseline_capture_sha256": sha(output / "capture.json"),
        "prefix_complete": True}
    save(output / "admission.json", sidecar)
    print(json.dumps({"output": str(output), "baseline": summary, "admission": sidecar}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--asm", type=Path, default=ROOT / "target/full-lifecycle-deps/asm-9.8.jar")
    parser.add_argument("--timeout", type=int, default=900)
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / "target") or output.exists():
        parser.error("output must be a new directory under target/")
    output.mkdir(parents=True)
    try:
        run(args, output)
    except Exception as error:
        save(output / "failure.json", {"type": type(error).__name__, "error": str(error),
            "command": getattr(error, "cmd", None), "returncode": getattr(error, "returncode", None)})
        raise


if __name__ == "__main__":
    main()
