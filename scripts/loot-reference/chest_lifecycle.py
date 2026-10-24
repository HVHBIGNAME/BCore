"""Research-only saved chest load/reopen oracle; use the ROOT heavy-check queue."""

import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import zipfile

from inventory import JAR, JAR_SHA256, ROOT, digest, write_json

HERE = Path(__file__).resolve().parent
JAVA = ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe"
ASM = ROOT / "target/native-environment-restoration-20261014-01/asm/asm-9.8.jar"
ASM_SHA256 = "876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051"
PRIOR = ROOT / "target/loot-native-20261016-fixtures"
PRIOR_FIXTURE_SHA256 = "2158fd855f1bbc46d6b399620c9ced35822263c982151686f58e175c1ccf9db1"
PRESET = '''{id:"minecraft:chest",x:8,y:80,z:8,Items:[
{Slot:0b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":7,"minecraft:enchantments":{"minecraft:sharpness":3}}},
{Slot:13b,id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:strong_healing"}}},
{Slot:26b,id:"minecraft:enchanted_book",count:1,components:{"minecraft:stored_enchantments":{"minecraft:wind_burst":1}}}
]}'''


def sha(path):
    return digest(path.read_bytes())


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def new_output(path):
    path = path.resolve()
    require(path.parent == ROOT / "target" and path.name.startswith("chest-lifecycle-native-20261018-"),
            "use a fresh target/chest-lifecycle-native-20261018-* directory")
    path.mkdir(exist_ok=False)
    return path


def freeze(source, destination):
    with destination.open("xb") as stream:
        stream.write(source.read_bytes())


def reader_module(out):
    source = ROOT / "scripts/jigsaw-reference/capture.py"
    frozen = out / "nbt_reader_reference.py"
    freeze(source, frozen)
    spec = importlib.util.spec_from_file_location("chest_native_nbt_reference", frozen)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def decoded_blob(blob, codec):
    raw = base64.b64decode(blob["base64"], validate=True)
    require(digest(raw) == blob["sha256"], "native NBT blob SHA mismatch")
    typed = codec.NbtReader(raw).root()
    return {**blob, "typed": typed}


def slot_projection(blob, codec):
    root = blob["typed"]["Compound"]
    rows = root.get("Items", {}).get("List", {}).get("values", [])
    slots = [None] * 27
    for row in rows:
        fields = row["Compound"]
        slot_kind, index = next(iter(fields["Slot"].items()))
        require(slot_kind == "Byte" and 0 <= index < 27, "expected native chest Slot Byte")
        slots[index] = {name: codec.plain(value) for name, value in fields.items() if name != "Slot"}
    return slots


def inspect(out):
    require(sha(PRIOR / "loot-fill-26.1.json") == PRIOR_FIXTURE_SHA256, "prior immutable loot fixture changed")
    codec = reader_module(out)
    data = load(PRIOR / "loot-fill-26.1.json")
    by_id = {case["id"]: case for case in data["cases"]}
    selected = ("dungeon-positive", "reward_ominous_rare-negative", "unregistered-fallback")
    reused, cases = [], []
    names = {"dungeon-positive": "deferred-dungeon-reopen", "reward_ominous_rare-negative": "deferred-book-reopen",
             "unregistered-fallback": "missing-table-reopen"}
    for key in selected:
        prior = by_id[key]
        unpack = prior["unpack"]
        before, after = (decoded_blob(unpack[field], codec) for field in ("saved_before", "saved_after"))
        require(slot_projection(after, codec) == prior["expected_slots"], "prior saved slots differ from observed item slots")
        require("Items" not in before["typed"]["Compound"], "prior deferred save unexpectedly has Items")
        require("LootTable" not in after["typed"]["Compound"] and "LootTableSeed" not in after["typed"]["Compound"],
                "prior materialized save unexpectedly retains loot metadata")
        reused.append({"prior_case": key, "authority": prior["authority"], "saved_before": before, "saved_after": after,
                       "memory_loot_seed_after": unpack["loot_seed_after"], "memory_loot_table_after": unpack["loot_table_after"],
                       "second_unpack_noop": unpack["second_unpack_noop"]})
        case = {"id": names[key], "input_kind": "prior_deferred", "base_nbt": unpack["saved_before"],
                "prior_case": key, "expected_table": prior["table"], "expected_seed_i64": prior["seed_i64"],
                "expected_native_empty": key == "unregistered-fallback"}
        if key != "unregistered-fallback":
            case["prior_after_nbt"] = unpack["saved_after"]
            case["prior_after_slots"] = prior["expected_slots"]
        else:
            case["reuse_limit"] = "Prior deferred save omits its preexisting diamond. Fresh load must not inherit that in-memory slot."
        cases.append(case)
    cases.insert(2, {"id": "preset-component-items", "input_kind": "preset", "snbt": PRESET,
                    "expected_table": None, "expected_seed_i64": "0"})
    mixed = dict(cases[0], id="table-and-items", input_kind="deferred_plus_items", items_snbt=PRESET)
    cases.insert(3, mixed)
    write_json(out / "inputs.json", {"schema": "bcore.native-chest-lifecycle.inputs.v1", "jar_sha256": JAR_SHA256,
               "world_seed_i64": "846692123413862008", "block_pos": [8, 80, 8], "player": None, "cases": cases})
    write_json(out / "reused-authority.json", {"fixture": str(PRIOR / "loot-fill-26.1.json"),
               "fixture_sha256": PRIOR_FIXTURE_SHA256, "prior_manifest_sha256": sha(PRIOR / "manifest.json"),
               "prior_independent_repeats_sha256": sha(PRIOR / "independent-repeats.json"), "cases": reused,
               "already_dynamic": ["Deferred save omits Items", "Materialized save omits LootTable/LootTableSeed",
                                   "In-memory seed retained", "Second unpack slots and original RNG state unchanged",
                                   "Missing registered key resolves through the native EMPTY table"],
               "requires_new_capture": ["Fresh materialized saved-NBT load and reopen", "Explicit Slot Byte preset component load",
                                        "Native table-plus-Items load precedence", "No fill/context/RNG calls during repeated/reopened unpack"]})
    for name in ("chest_lifecycle.py", "inventory.py"):
        freeze(HERE / name, out / name)
    write_json(out / "manifest.json", {p.name: sha(p) for p in out.iterdir() if p.is_file()})
    print(json.dumps({"output": str(out), "reused_cases": len(reused), "new_cases": len(cases), "inputs_sha256": sha(out / "inputs.json")}))


def capture(out, args):
    from chest_lifecycle_verify import verify

    require(args.inputs is not None, "capture requires the frozen --inputs from inspect")
    require(sha(JAR) == JAR_SHA256 and sha(ASM) == ASM_SHA256, "pinned native JAR/ASM mismatch")
    require(sha(PRIOR / "loot-fill-26.1.json") == PRIOR_FIXTURE_SHA256, "prior native fixture changed")
    spec = load(args.inputs)
    require(spec["schema"] == "bcore.native-chest-lifecycle.inputs.v1" and spec["jar_sha256"] == JAR_SHA256,
            "wrong lifecycle input schema or JAR")
    require(spec["player"] is None and 1 <= len(spec["cases"]) <= 12, "bounded null-player cases required")
    for case in spec["cases"]:
        if case["expected_table"] is not None:
            require(int(case["expected_seed_i64"]) != 0, "seed-zero generation is outside this lane")
    javac = shutil.which("javac")
    require(javac is not None, "javac21 must be on PATH")
    build, server_dir = out / "build", out / "server"
    build.mkdir()
    server_dir.mkdir()
    codec = reader_module(build)
    sources = [HERE / "ChestLifecycleAgent.java", HERE / "ChestLifecycleProbe.java",
               ROOT / "scripts/native-generation-reference/NativeAccess.java"]
    scripts = [HERE / "chest_lifecycle.py", HERE / "chest_lifecycle_verify.py", HERE / "inventory.py"]
    for source in [*sources, *scripts]:
        freeze(source, build / source.name)
    freeze(ASM, build / ASM.name)
    freeze(args.inputs, build / "inputs.json")
    config = {**spec, "output": str(out)}
    write_json(out / "config.json", config)
    frozen_hashes = {str(source.relative_to(ROOT)): sha(build / source.name) for source in [*sources, *scripts]}
    if args.repeat_of:
        prior = load(args.repeat_of / "provenance.json")
        require(frozen_hashes == prior["sources"] and sha(args.inputs) == prior["inputs_sha256"],
                "independent repeat requires identical frozen sources and inputs")
    compile_command = [javac, "--release", "21", "-cp", str(build / ASM.name), "-d", str(build),
                       *[str(build / source.name) for source in sources]]
    with (out / "compile.log").open("x", encoding="utf-8") as log:
        subprocess.run(compile_command, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=120)
    agent = build / "chest-lifecycle-agent.jar"
    with zipfile.ZipFile(agent, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: ChestLifecycleAgent\n\n")
        for path in sorted(build.glob("*.class")):
            archive.write(path, path.name)
    properties = "\n".join(("level-name=chest-lifecycle-world", f"level-seed={spec['world_seed_i64']}",
        "level-type=minecraft:normal", "generate-structures=true", "server-ip=127.0.0.1", "server-port=0", "online-mode=false",
        "enable-rcon=false", "enable-query=false", "management-server-enabled=false", "sync-chunk-writes=false",
        "max-tick-time=-1", "view-distance=2", "simulation-distance=2", "pause-when-empty-seconds=0", "enforce-secure-profile=false", ""))
    (server_dir / "server.properties").write_text(properties, encoding="utf-8")
    (server_dir / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    libraries = sorted((ROOT / "target/vanilla-775/libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [agent, build / ASM.name, JAR, *libraries]))
    command = [str(JAVA), "-Xmx2G", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
               f"-javaagent:{agent}={out / 'config.json'}", "-cp", classpath, "net.minecraft.server.Main", "--nogui"]
    provenance = {"schema": 1, "jar": str(JAR), "jar_sha256": sha(JAR), "asm_sha256": sha(ASM),
        "java": subprocess.run([str(JAVA), "-version"], capture_output=True, text=True, check=True, timeout=30).stderr,
        "javac": subprocess.run([javac, "-version"], capture_output=True, text=True, check=True, timeout=30).stdout,
        "java_executable_sha256": sha(JAVA), "javac_executable_sha256": sha(Path(javac)),
        "sources": frozen_hashes, "nbt_decoder_sha256": sha(build / "nbt_reader_reference.py"),
        "inputs_source": str(args.inputs.resolve()), "inputs_sha256": sha(args.inputs),
        "prior_fixture_sha256": PRIOR_FIXTURE_SHA256, "config_sha256": sha(out / "config.json"),
        "compiled_classes": {p.name: sha(p) for p in sorted(build.glob("*.class"))}, "agent_sha256": sha(agent),
        "compile_command": compile_command, "command": command, "server_properties": properties,
        "libraries": {str(p.relative_to(ROOT)): sha(p) for p in libraries},
        "queue_environment": {key: os.environ.get(key) for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "RAYON_NUM_THREADS")},
        "authority": "Original BlockEntity.loadStatic/loadWithComponents/ChestBlockEntity load/save/unpack on a real native ServerLevel and fresh registered chests.",
        "limits": ["Null-player unpack and getItem; no actual player menu", "Seed0 fill is explicitly rejected", "Zero gameplay ticks", "No process restart/chunk-region round-trip claim"]}
    write_json(out / "provenance.json", provenance)
    with (out / "server.log").open("x", encoding="utf-8") as log:
        completed = subprocess.run(command, cwd=server_dir, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
    require(completed.returncode == 0 and (out / "observations.raw.json").exists(), f"native lifecycle capture failed; inspect {out / 'server.log'}")
    require("Serialization errors:" not in (out / "server.log").read_text(encoding="utf-8"), "native serialization errors")
    instrumentation = load(out / "instrumentation.json")
    with zipfile.ZipFile(JAR) as archive:
        for row in instrumentation["classes"]:
            require(digest(archive.read(row["class"] + ".class")) == row["original_sha256"], "observed class differs from pinned JAR")
    require(sha(JAR) == JAR_SHA256 and sha(PRIOR / "loot-fill-26.1.json") == PRIOR_FIXTURE_SHA256, "immutable authority changed")
    if args.repeat_of:
        prior = load(args.repeat_of / "provenance.json")
        for key in ("java_executable_sha256", "javac_executable_sha256", "compiled_classes", "libraries", "server_properties", "nbt_decoder_sha256"):
            require(provenance[key] == prior[key], f"repeat environment differs: {key}")
    verified = verify(out, spec, codec, args.repeat_of)
    write_json(out / "manifest.json", {p.relative_to(out).as_posix(): sha(p) for p in sorted(out.rglob("*")) if p.is_file()})
    print(json.dumps({"output": str(out), **verified}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("inspect", "capture"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--inputs", type=Path)
    parser.add_argument("--repeat-of", type=Path)
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    require(os.environ.get("CARGO_TARGET_DIR") == str(ROOT / "target/build") and os.environ.get("CARGO_BUILD_JOBS") == "1",
            "run through ROOT scripts/run_generation_check.py --background LABEL -- COMMAND")
    out = new_output(args.output)
    if args.mode == "inspect":
        inspect(out)
    else:
        capture(out, args)


if __name__ == "__main__":
    main()
