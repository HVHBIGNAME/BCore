"""Capture 26.2 production catalogs into fresh target directories on main's queue."""
from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import argparse
import base64
from dataclasses import dataclass
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
import zipfile

import native

COMMON = ("scripts/TreeReference.java", "scripts/NativeWorldgenRegistries.java")
BASE = (*COMMON, "scripts/BaseFeatureReference.java")
VEGETATION = (*COMMON, "scripts/OreReference.java", "scripts/VegetationReference.java")
JIGSAW = (*COMMON, "scripts/jigsaw-reference/JigsawSupport.java", "scripts/jigsaw-reference/JigsawReference.java")
STRUCTURES = (*JIGSAW, "scripts/structure-runtime-reference/StructureRuntimeReference.java")
SCATTERED = (*STRUCTURES, "scripts/scattered-structure-reference/ScatteredStructureReference.java")
WRAPPER = "scripts/migration-26.2/CatalogReference.java"
SPAWN = ("scripts/TreeReference.java", "scripts/native-generation-reference/NativeAccess.java",
         "scripts/generation-spawn-reference/GenerationSpawnProbe.java",
         "scripts/generation-spawn-reference/GenerationSpawnHandoff.java",
         "scripts/generation-spawn-reference/GenerationSpawnInputs.java",
         "scripts/migration-26.2/CatalogSpawnAgent.java", "scripts/migration-26.2/SpawnCatalogReference.java")
FAMILIES = ("ancient_city", "village", "trail_ruins", "trial_chambers", "pillager_outpost")
ASM = native.ROOT / "target/native-environment-restoration-20261014-01/asm/asm-9.8.jar"
ASM_SHA256 = "876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051"
NBT_LIMIT = 4 * 1024 * 1024
RESOURCE_LIMIT = 64 * 1024 * 1024


@dataclass(frozen=True)
class Probe:
    launcher: str
    sources: tuple[str, ...]
    filename: str
    marker: str = "CATALOG_REFERENCE="
    arguments: tuple[str, ...] = ()
    projection: str = "identity"
    timeout: int = 600
    heap: str = "1G"
    environment: tuple[tuple[str, str | None], ...] = ()


def wrapped(mode: str, sources: tuple[str, ...], filename: str, *, projection: str = "identity") -> Probe:
    return Probe("CatalogReference", (*sources, WRAPPER), filename, arguments=(mode, "{jar}"), projection=projection)


PROBES = {
    "heightmap": Probe("HeightmapReference", (*COMMON, "scripts/HeightmapReference.java"),
                       "heightmaps_26_2.json", "HEIGHTMAP_REFERENCE="),
    "standing_blocks": wrapped("standing_blocks", VEGETATION, "standing_blocks_26_2.json"),
    "monster_room": Probe("MonsterRoomReference", (*COMMON, "scripts/MonsterRoomReference.java"),
                          "monster_rooms_26_2.json", "MONSTER_ROOM_REFERENCE="),
    "mineshaft_blocks": wrapped("mineshaft_blocks", (*COMMON, "scripts/OreReference.java",
                                "scripts/NativeEntityLevel.java", "scripts/MineshaftBlockReference.java"),
                                "mineshaft_blocks_26_2.json"),
    "mineshaft_start": wrapped("mineshaft_start", (*COMMON, "scripts/OreReference.java",
                              "scripts/MineshaftStartReference.java"), "mineshaft_starts_26_2.json"),
    "base_catalog": Probe("BaseFeatureReference", BASE, "base_feature_catalog_26_2.json",
                          "BASE_FEATURE_REFERENCE=", ("catalog", "{jar}"), timeout=900),
    "misc_states": Probe("MiscFeatureReference", (*BASE, "crates/bcore-worldgen/probes/misc_features/MiscFeatureReference.java"),
                         "misc_feature_states_26_2.json", "MISC_FEATURE_REFERENCE=", ("metadata", "{jar}", "priority")),
    "cave_blocks": Probe("CaveFeatureReference", (*COMMON, "scripts/CaveFeatureReference.java"),
                         "cave_feature_data_26_2.json", "CAVE_FEATURE_REFERENCE=", ("blocks",)),
    "sculk_states": Probe("SculkReference", (*COMMON, "scripts/NativeEntityLevel.java",
                          "scripts/TreeEffectReference.java", "scripts/SculkReference.java"),
                          "sculk_states_26_2.json", "SCULK_REFERENCE=", ("--data-only",), "sculk"),
    "surface_blocks": wrapped("surface_blocks", (*COMMON, "scripts/SurfaceBuilderReference.java"),
                              "surface_builder_blocks_26_2.json"),
    "surface_climates": Probe("CatalogReference", (*COMMON, WRAPPER), "surface_climates_26_2.json",
                              arguments=("surface_climates", "{jar}"), projection="climates"),
    "extra_tree": Probe("ExtraTreeReference", (*VEGETATION, "scripts/ExtraTreeReference.java"),
                        "extra_tree_catalog_26_2.json", "EXTRA_TREE_REFERENCE=",
                        environment=(("BCORE_EXTRA_TREE_PROBE", None),)),
    "falling_shapes": Probe("FallingShapeReference", ("scripts/TreeReference.java", "scripts/FallingShapeReference.java"),
                            "falling_block_shapes_26_2.json", "FALLING_SHAPE_REFERENCE="),
    "lighting_catalog": Probe("LightingReference", (*BASE, "scripts/lighting-reference/LightingReference.java"),
                              "lighting_catalog_26_2.json", "LIGHTING_REFERENCE=", ("catalog", "{jar}"), timeout=900),
    "lighting_updates": Probe("LightingReference", (*BASE, "scripts/lighting-reference/LightingReference.java"),
                              "lighting_updates_26_2.json", "LIGHTING_REFERENCE=", ("updates", "{jar}")),
    "scattered_catalog": wrapped("scattered_catalog", SCATTERED, "scattered_structure_catalog_26_2.json"),
    "pyramid_catalog": wrapped("pyramid_catalog", (*SCATTERED, "scripts/desert-pyramid-reference/DesertPyramidReference.java"),
                              "desert_pyramid_catalog_26_2.json"),
    "scattered_containers": wrapped("scattered_containers", SCATTERED, "scattered_containers_26_2.json", projection="nbt"),
    "brushable": Probe("BrushableRuntimeReference", (*STRUCTURES, "scripts/structure-runtime-reference/BrushableRuntimeReference.java"),
                       "brushable_block_entities_26_2.json", "STRUCTURERUNTIMEREFERENCE=", projection="nbt"),
    "jigsaw_assets": Probe("PoolAliasTrialAssetsBootstrap", (*JIGSAW, "scripts/jigsaw-reference/JigsawAssets.java",
                           "scripts/pool-alias-trial-reference/PoolAliasTrialAssetsBootstrap.java"),
                           "pool_alias_trial_combined_assets_26_2.json", "JIGSAWASSETS=", ("{jar}", "{families}"),
                           "jigsaw", timeout=1200, heap="3G"),
    "fossil_assets": Probe("FossilAssets", (*COMMON, "scripts/jigsaw-reference/JigsawSupport.java",
                           "scripts/fossil-reference/FossilAssets.java"), "fossil_assets_26_2.json",
                           "FOSSILASSETS=", projection="fossil"),
    "trig_table": Probe("", (), "misc_feature_trig_26_2.json", projection="math"),
    "spawn_assets": Probe("net.minecraft.server.Main", SPAWN, "generation_spawn_assets_26_2.json",
                          projection="spawn", timeout=1200, heap="2G"),
    "spawn_callbacks": Probe("net.minecraft.server.Main", SPAWN, "generation_spawn_callbacks_26_2.json",
                             projection="spawn", timeout=1200, heap="2G"),
    "spawn_handoff": Probe("net.minecraft.server.Main", SPAWN, "generation_mob_catalog_26_2.json",
                           projection="handoff", timeout=1200, heap="2G"),
}


def snapshot(source: Path, destination: Path) -> str:
    content = source.read_bytes()
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("xb") as stream:
        stream.write(content)
    digest = hashlib.sha256(content).hexdigest()
    if native.sha(source) != digest or native.sha(destination) != digest:
        raise ValueError(f"source changed during snapshot: {source}")
    return digest


def freeze_sources(output: Path, probe: Probe) -> dict[str, str]:
    helpers = ["scripts/migration-26.2/catalogs.py", "scripts/migration-26.2/native.py"]
    if probe.projection in ("nbt", "jigsaw"):
        helpers.append("scripts/jigsaw-reference/capture.py")
    if probe.projection == "math":
        helpers.append("crates/bcore-worldgen/probes/misc_features/build_trig_table.py")
    paths = (*probe.sources, *helpers)
    if len({Path(name).name for name in probe.sources}) != len(probe.sources):
        raise ValueError("duplicate Java source filenames")
    return {name: snapshot(native.ROOT / name, output / "sources" / name) for name in paths}


def preserved_source_hashes(sources: dict[str, str]) -> dict[str, str]:
    prepared = json.loads(native.PREPARATION.read_bytes())
    baseline = prepared["baseline_snapshot"]["source_sha256"]
    pins = {name: baseline[name] for name in sources if name in baseline}
    for name, digest in pins.items():
        if sources[name] != digest and "/migration-26.2/" not in name:
            raise ValueError(f"historical probe/helper changed from the preparation snapshot: {name}")
    return pins


def frozen_module(output: Path, name: str):
    path = output / "sources" / name
    spec = importlib.util.spec_from_file_location("migration_catalog_" + path.stem, path)
    if spec is None or spec.loader is None:
        raise ValueError(f"cannot load frozen helper: {name}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def bounded_execute(command: list[str], output: Path, name: str, *, timeout: int,
                    log_limit: int, environment: dict[str, str]) -> Path:
    native.write_json(output / f"{name}.command.json", {"command": command, "cwd": str(output),
                      "timeout_seconds": timeout, "stream_limit_bytes": log_limit})
    overflow = threading.Event()
    errors: list[str] = []
    counts = {"stdout": 0, "stderr": 0}

    def drain(pipe, stream, channel):
        try:
            with pipe:
                for block in iter(lambda: pipe.read(65536), b""):
                    remaining = max(0, log_limit - counts[channel])
                    stream.write(block[:remaining])
                    counts[channel] += len(block)
                    if counts[channel] > log_limit:
                        overflow.set()
        except OSError as error:
            errors.append(f"{channel}: {error}")
            overflow.set()

    started = time.monotonic()
    failure = None
    stdout = output / f"{name}.stdout.log"
    stderr = output / f"{name}.stderr.log"
    with stdout.open("xb") as out, stderr.open("xb") as err:
        with subprocess.Popen(command, cwd=output, env=environment, stdin=subprocess.DEVNULL,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE) as process:
            threads = [threading.Thread(target=drain, args=(process.stdout, out, "stdout")),
                       threading.Thread(target=drain, args=(process.stderr, err, "stderr"))]
            for thread in threads:
                thread.start()
            try:
                while process.poll() is None:
                    if overflow.wait(0.05):
                        failure = "subprocess output exceeded its bound or could not be retained"
                        break
                    if time.monotonic() - started > timeout:
                        failure = f"subprocess exceeded {timeout} seconds"
                        break
            finally:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=30)
                for thread in threads:
                    thread.join()
            returncode = process.returncode
    native.write_json(output / f"{name}.result.json", {"exit_code": returncode,
                      "elapsed_seconds": time.monotonic() - started, "observed_stream_bytes": counts,
                      "stdout_sha256": native.sha(stdout), "stderr_sha256": native.sha(stderr),
                      "errors": errors, "failure": failure, "truncated": overflow.is_set()})
    if failure or overflow.is_set():
        raise RuntimeError(f"{name}: {failure or errors}; retained bounded logs in {output}")
    if returncode:
        raise subprocess.CalledProcessError(returncode, command)
    return stdout


def read_result(stdout: Path, marker: str, limit: int) -> dict:
    if stdout.stat().st_size > limit:
        raise ValueError("native output exceeds the admitted bound")
    text = stdout.read_text(encoding="utf-8")
    stderr = stdout.with_name(stdout.name.replace(".stdout.log", ".stderr.log"))
    if "Serialization errors:" in text or "Serialization errors:" in stderr.read_text(encoding="utf-8"):
        raise ValueError("native serialization failed; retained original logs")
    records = [line.split(marker, 1)[1] for line in text.splitlines() if marker in line]
    if len(records) != 1:
        raise ValueError(f"expected exactly one {marker} record, observed {len(records)}")
    value = json.loads(records[0])
    if not isinstance(value, dict):
        raise ValueError("catalog result must be an object")
    for key, expected in (("minecraft", "26.2"), ("protocol", 776), ("jar_sha256", native.JAR_SHA256)):
        if key in value and value[key] != expected:
            raise ValueError(f"native result carries a different {key}; refusing to relabel it")
    return value


def nbt(codec, value: str) -> dict:
    if len(value) > (NBT_LIMIT + 2) // 3 * 4:
        raise ValueError("native NBT exceeds the per-object bound")
    raw = base64.b64decode(value, validate=True)
    if len(raw) > NBT_LIMIT:
        raise ValueError("native NBT exceeds the per-object bound")
    return codec.NbtReader(raw).root()


def decode_nbt(codec, value):
    if isinstance(value, dict):
        return {key: nbt(codec, child) if key == "nbt" and isinstance(child, str) else decode_nbt(codec, child)
                for key, child in value.items()}
    if isinstance(value, list):
        return [decode_nbt(codec, child) for child in value]
    return value


def jar_resources(select) -> dict:
    resources = {}
    total = 0
    with zipfile.ZipFile(native.JAR) as archive:
        for entry in archive.infolist():
            if not entry.filename.endswith(".json") or not select(entry.filename):
                continue
            total += entry.file_size
            if entry.file_size > NBT_LIMIT or total > RESOURCE_LIMIT:
                raise ValueError("selected JAR resources exceed the bounded projection")
            raw = archive.read(entry)
            resources[entry.filename] = {"sha256": hashlib.sha256(raw).hexdigest(), "value": json.loads(raw)}
    return resources


def project_jigsaw(data: dict, output: Path, families: tuple[str, ...]) -> dict:
    codec = frozen_module(output, "scripts/jigsaw-reference/capture.py")
    result = dict(data)
    result["block_entities"] = {name: {**row, "nbt": nbt(codec, row["nbt"])}
                                for name, row in data["block_entities"].items()}
    templates = {}
    for name, encoded in result.pop("template_nbt").items():
        root = nbt(codec, encoded)["Compound"]
        plain = codec.plain({"Compound": root})
        blocks = []
        for typed, block in zip(root["blocks"]["List"]["values"], plain["blocks"], strict=True):
            row = [*block["pos"], block["state"]]
            if "nbt" in typed["Compound"]:
                row.append(typed["Compound"]["nbt"])
            blocks.append(row)
        entities = [{"pos": entity["pos"], "block_pos": entity["blockPos"], "nbt": typed["Compound"]["nbt"]}
                    for typed, entity in zip(root["entities"]["List"]["values"], plain["entities"], strict=True)]
        templates[name] = {"size": plain["size"], "palettes": plain.get("palettes", [plain.get("palette", [])]),
                           "blocks": blocks, "entities": entities, "data_version": plain["DataVersion"]}
    result["templates"] = templates
    structures = {name.removeprefix("minecraft:") for name in data["structure_metadata"]}
    set_names = {"ancient_city": "ancient_cities", "village": "villages", "pillager_outpost": "pillager_outposts"}
    sets = {set_names.get(family, family) for family in families}

    def select(path):
        return (path.startswith("data/minecraft/worldgen/template_pool/")
                and (any(f"/{family}/" in path for family in families) or path.endswith("/empty.json"))
                or path.startswith("data/minecraft/worldgen/processor_list/")
                or path.startswith("data/minecraft/worldgen/structure/") and Path(path).stem in structures
                or path.startswith("data/minecraft/worldgen/structure_set/") and Path(path).stem in sets
                or path.startswith("data/minecraft/tags/block/")
                or path.startswith("data/minecraft/tags/worldgen/biome/has_structure/") and Path(path).stem in structures
                or path in ("data/minecraft/worldgen/placed_feature/sculk_patch_ancient_city.json",
                            "data/minecraft/worldgen/configured_feature/sculk_patch_ancient_city.json"))

    result["jar_assets"] = jar_resources(select)
    with zipfile.ZipFile(native.JAR) as archive:
        sources = {}
        for name in templates:
            path = f"data/minecraft/structure/{name.removeprefix('minecraft:')}.nbt"
            missing = name in data["missing_templates"]
            sources[name] = {"path": path, "missing_from_jar": missing,
                             "sha256": None if missing else hashlib.sha256(archive.read(path)).hexdigest()}
    result["template_sources"] = sources
    return result


def project(data: dict, probe: Probe, output: Path, families: tuple[str, ...]) -> dict:
    if probe.projection == "sculk":
        return data["data"]
    if probe.projection == "climates":
        return data["climates"]
    if probe.projection == "handoff":
        return data["handoff_catalog"]
    if probe.projection == "nbt":
        return decode_nbt(frozen_module(output, "scripts/jigsaw-reference/capture.py"), data)
    if probe.projection == "jigsaw":
        return project_jigsaw(data, output, families)
    if probe.projection == "fossil":
        result = dict(data)
        with zipfile.ZipFile(native.JAR) as archive:
            result["template_sources"] = {name: hashlib.sha256(archive.read(
                f"data/minecraft/structure/{name.removeprefix('minecraft:')}.nbt")).hexdigest() for name in data["templates"]}
        resources = jar_resources(lambda path: path in {
            f"data/minecraft/worldgen/processor_list/{name}.json" for name in ("fossil_rot", "fossil_coal", "fossil_diamonds")})
        result["processor_sources"] = {Path(path).stem: row for path, row in resources.items()}
        return result
    return data


def validate_ranges(rows, label: str) -> None:
    end = 0
    for row in rows:
        if row[0] != end or row[1] <= end or row[1] > 32366:
            raise ValueError(f"{label}: incomplete or overlapping native state ranges")
        end = row[1]
    if end != 32366:
        raise ValueError(f"{label}: expected ranges covering 32366 target states")


def validate_catalog(mode: str, data: dict) -> None:
    ranges = {"heightmap": "ranges", "standing_blocks": "ranges", "monster_room": "ranges",
              "mineshaft_blocks": "ranges", "base_catalog": "state_ranges", "misc_states": "shape_ranges",
              "cave_blocks": "state_ranges", "sculk_states": "ranges", "lighting_catalog": "state_ranges"}
    if mode in (*ranges, "surface_blocks", "falling_shapes") and data["state_count"] != 32366:
        raise ValueError(f"{mode}: expected 32366 target states")
    if mode in ranges:
        validate_ranges(data[ranges[mode]], mode)
    if mode == "base_catalog":
        if len(data["biome_ids"]) != 66 or set(data["biome_ids"].values()) != set(range(66)):
            raise ValueError("base catalog must include all 66 native biome IDs")
    if mode == "surface_climates" and len(data) != 66:
        raise ValueError("surface climate catalog must include all 66 native biomes")
    if mode == "lighting_catalog" and len(data["climates"]) != 66:
        raise ValueError("light catalog must include all 66 native biomes")
    if mode == "scattered_catalog" and data["block_properties"]["state_count"] != 32366:
        raise ValueError("scattered predicates must include all 32366 states")
    if mode == "scattered_catalog":
        validate_ranges(data["block_properties"]["ranges"], mode)
    if mode == "extra_tree":
        validate_ranges(sorted(([row["first"], row["end"]] for row in data["blocks"].values()), key=lambda row: row[0]), mode)
    if mode == "jigsaw_assets":
        if len(data["states"]) != 32366 or any(row[0] != index for index, row in enumerate(data["states"])):
            raise ValueError("jigsaw state table is not the complete ordered target registry")
    if mode == "spawn_assets" and data["blocks"]["state_count"] != 32366:
        raise ValueError("spawn predicates must include all 32366 states")
    if mode == "spawn_assets":
        validate_ranges(data["blocks"]["runs"], mode)
    if mode in ("spawn_assets", "spawn_callbacks"):
        if data["gameplay_ticks"] != 0 or len(data["catalog"]["biomes"]) != 66:
            raise ValueError("spawn catalog must be a zero-gameplay-tick capture of all 66 native biomes")
        for name, row in data["constructors"].items():
            if row["entity"]["problems"]:
                raise ValueError(f"native constructor serialization failed: {name}")


def compile_probe(probe: Probe, output: Path, javac: str, asm: Path | None, execute) -> Path:
    build = output / "classes"
    build.mkdir()
    command = [javac, f"-J-Djava.io.tmpdir={output / 'tmp'}", "--release", "21", "-encoding", "UTF-8",
               "-implicit:none", "-d", str(build)]
    command.extend(["-cp", os.pathsep.join(map(str, [build, asm] if asm is not None else [build]))])
    command.extend(str(output / "sources" / path) for path in probe.sources)
    execute(command, "compile", timeout=120)
    return build


def spawn_command(mode: str, probe: Probe, output: Path, build: Path,
                  libraries: list[Path], asm: Path, seed: int) -> list[str]:
    agent = output / "catalog-spawn-agent.jar"
    with zipfile.ZipFile(agent, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nPremain-Class: CatalogSpawnAgent\n\n")
        for path in sorted(build.glob("*.class")):
            archive.write(path, path.name)
    config = output / "spawn-config.json"
    native.write_json(config, {"output": str(output), "mode": mode, "seed": str(seed),
                             "minecraft": "26.2", "protocol": 776})
    properties = "\n".join(("level-name=catalog-world", f"level-seed={seed}", "level-type=minecraft:normal",
                            "generate-structures=true", "server-ip=127.0.0.1", "server-port=0", "online-mode=false",
                            "enable-rcon=false", "enable-query=false", "management-server-enabled=false",
                            "sync-chunk-writes=false", "max-tick-time=-1", "view-distance=2", "simulation-distance=2",
                            "pause-when-empty-seconds=0", "enforce-secure-profile=false", ""))
    for name, text in (("server.properties", properties), ("eula.txt", "eula=true\n")):
        with (output / name).open("x", encoding="utf-8") as stream:
            stream.write(text)
    classpath = os.pathsep.join(map(str, [agent, asm, native.JAR, *libraries]))
    return [str(native.JAVA), f"-Xmx{probe.heap}", "-XX:ActiveProcessorCount=2", "-Dmax.bg.threads=1",
            f"-Djava.io.tmpdir={output / 'tmp'}",
            f"-javaagent:{agent}={config}", "-cp", classpath, probe.launcher, "--nogui"]


def capture(mode: str, output: Path, libraries: list[Path], javac: str,
            families: tuple[str, ...], log_limit: int, timeout: int | None,
            asm: Path | None, seed: int) -> dict:
    probe = PROBES[mode]
    output.mkdir()
    sources = freeze_sources(output, probe)
    environment = dict(os.environ)
    temporary = output / "tmp"
    temporary.mkdir()
    environment.update({key: str(temporary) for key in ("TMP", "TEMP", "TMPDIR")})
    for name, value in probe.environment:
        if value is None:
            environment.pop(name, None)
        else:
            environment[name] = value
    provenance = {"schema": "bcore.migration.catalog-capture.v1", "mode": mode, "minecraft": "26.2",
                  "protocol": 776, "jar_sha256": native.JAR_SHA256, "source_sha256": sources,
                  "historical_source_sha256": preserved_source_hashes(sources),
                  "java_source_order": list(probe.sources), "marker": probe.marker,
                  "controlled_environment": dict(probe.environment), "families": list(families) if mode == "jigsaw_assets" else [],
                  "temporary_directory": str(temporary),
                  "projection": probe.projection,
                  "authority": "derived-mathematical-input" if probe.projection == "math" else "native-catalog",
                  "environment_sha256": native.sha(output.parent / "environment.json")}
    native.write_json(output / "capture-inputs.json", provenance)

    def execute(command, name, *, timeout):
        return bounded_execute(command, output, name, timeout=timeout, log_limit=log_limit, environment=environment)

    if probe.projection == "math":
        generator = frozen_module(output, "crates/bcore-worldgen/probes/misc_features/build_trig_table.py")
        raw = generator.make_table()
        raw["generator_sha256"] = sources["crates/bcore-worldgen/probes/misc_features/build_trig_table.py"]
    else:
        spawn = mode.startswith("spawn_")
        build = compile_probe(probe, output, javac, asm if spawn else None, execute)
        if spawn:
            if asm is None:
                raise ValueError("spawn metadata requires an authenticated ASM JAR")
            command = spawn_command(mode, probe, output, build, libraries, asm, seed)
        else:
            arguments = []
            for argument in probe.arguments:
                arguments.extend(families if argument == "{families}" else [argument.replace("{jar}", str(native.JAR))])
            command = [str(native.JAVA), f"-Xmx{probe.heap}", "-XX:ActiveProcessorCount=2",
                       f"-Djava.io.tmpdir={temporary}", "-cp",
                       os.pathsep.join(map(str, [build, native.JAR, *libraries])), probe.launcher, *arguments]
        stdout = execute(command, "capture", timeout=timeout or probe.timeout)
        raw = read_result(stdout, probe.marker, log_limit)
    native.write_json(output / "native-result.json", raw)
    data = project(raw, probe, output, families)
    if not isinstance(data, dict):
        raise ValueError("catalog projection must be an object")
    validate_catalog(mode, data)
    if probe.projection != "climates":
        data = {**data, "minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256}
        if probe.sources:
            data["probe_sha256"] = hashlib.sha256(b"".join(
                (output / "sources" / name).read_bytes() for name in probe.sources)).hexdigest()
    native.write_json(output / probe.filename, data)
    for name, digest in sources.items():
        if native.sha(native.ROOT / name) != digest:
            raise ValueError(f"source changed during capture: {name}")
        if native.sha(output / "sources" / name) != digest:
            raise ValueError(f"frozen source changed during capture: {name}")
    if native.sha(native.JAR) != native.JAR_SHA256:
        raise ValueError("target JAR changed during capture")
    files = {path.relative_to(output).as_posix(): native.sha(path)
             for path in sorted(output.rglob("*"))
             if path.is_file() and path.relative_to(output).parts[0] != "catalog-world"}
    native.write_json(output / "manifest.json", {**provenance, "files_sha256": files,
                      "catalog": probe.filename, "catalog_sha256": files[probe.filename]})
    return {"status": "pass", "directory": str(output.relative_to(native.ROOT)),
            "catalog": probe.filename, "catalog_sha256": files[probe.filename],
            "manifest_sha256": native.sha(output / "manifest.json")}


def capture_environment(output: Path, libraries: list[Path], version: dict, javac: str,
                        asm: Path | None, asm_source: Path | None, log_limit: int) -> dict:
    environment = dict(os.environ)
    temporary = output / "tmp"
    temporary.mkdir()
    environment.update({key: str(temporary) for key in ("TMP", "TEMP", "TMPDIR")})
    for name, command in (("java-version", [str(native.JAVA), f"-Djava.io.tmpdir={temporary}", "-version"]),
                          ("javac-version", [javac, f"-J-Djava.io.tmpdir={temporary}", "-version"])):
        bounded_execute(command, output, name, timeout=30, log_limit=log_limit, environment=environment)
    evidence = {"version": version, "jar_sha256": native.JAR_SHA256,
                "preparation_sha256": native.sha(native.PREPARATION), "java_executable_sha256": native.sha(native.JAVA),
                "javac_executable_sha256": native.sha(Path(javac)), "python_executable_sha256": native.sha(Path(sys.executable)),
                "driver_sha256": native.sha(Path(__file__)), "native_helper_sha256": native.sha(Path(native.__file__)),
                "version_evidence_sha256": {path.name: native.sha(path) for path in sorted(output.glob("*-version.*"))},
                "libraries": {path.relative_to(native.ROOT).as_posix(): native.sha(path) for path in libraries},
                "runtime_environment": {key: environment.get(key) for key in
                    ("JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS", "CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "RAYON_NUM_THREADS")},
                "asm": None if asm is None else {"path": str(asm), "source": str(asm_source), "sha256": native.sha(asm)}}
    native.write_json(output / "environment.json", evidence)
    return evidence


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("modes", nargs="+", choices=(*PROBES, "all", "list"))
    parser.add_argument("--output", type=Path, help="absent directory whose parent exists under target/")
    parser.add_argument("--family", action="append", choices=(*FAMILIES, "spring"))
    parser.add_argument("--keep-going", action="store_true", help="retain failures and continue independent selected modes")
    parser.add_argument("--timeout", type=int, help="per-capture timeout override, seconds")
    parser.add_argument("--max-log-mib", type=int, default=64, help="maximum retained bytes per subprocess stream, MiB")
    parser.add_argument("--seed", type=int, default=846692123413862008)
    parser.add_argument("--asm", type=Path, default=ASM)
    parser.add_argument("--asm-sha256", default=ASM_SHA256)
    args = parser.parse_args()
    if args.modes == ["list"]:
        print(json.dumps({name: {"catalog": probe.filename, "launcher": probe.launcher,
                               "marker": probe.marker, "timeout_seconds": probe.timeout}
                          for name, probe in PROBES.items()}, indent=2))
        return
    if "list" in args.modes or "all" in args.modes and args.modes != ["all"]:
        parser.error("list and all must each be used alone")
    if args.output is None or not 1 <= args.max_log_mib <= 256 or args.timeout is not None and args.timeout <= 0:
        parser.error("provide --output, a 1..256 MiB log bound and a positive timeout")
    if not -(1 << 63) <= args.seed < 1 << 63:
        parser.error("seed must be signed i64")
    output = args.output.resolve()
    if output.exists() or not output.parent.is_dir() or not output.is_relative_to(native.ROOT / "target"):
        parser.error("output must be absent with an existing parent under target/")
    modes = list(PROBES) if args.modes == ["all"] else args.modes
    if len(set(modes)) != len(modes):
        parser.error("duplicate capture mode")
    if args.family and "jigsaw_assets" not in modes:
        parser.error("--family applies only to jigsaw_assets")
    version, libraries = native.environment()
    if len(libraries) != 39:
        raise ValueError("expected exactly 39 authenticated official 26.2 libraries")
    javac = shutil.which("javac")
    if javac is None:
        raise ValueError("javac 21+ is missing")
    asm = None
    if any(mode.startswith("spawn_") for mode in modes):
        asm = args.asm.resolve()
        if native.sha(asm) != args.asm_sha256:
            raise ValueError("ASM digest differs from the supplied pin")
    output.mkdir()
    asm_source = asm
    if asm is not None:
        frozen_asm = output / "tools" / asm.name
        if snapshot(asm, frozen_asm) != args.asm_sha256:
            raise ValueError("ASM changed while freezing the tool artifact")
        asm = frozen_asm
    log_limit = args.max_log_mib * 1024 * 1024
    evidence = capture_environment(output, libraries, version, javac, asm, asm_source, log_limit)
    results = {}
    for mode in modes:
        try:
            results[mode] = capture(mode, output / mode, libraries, javac,
                                    tuple(args.family or FAMILIES), log_limit, args.timeout, asm, args.seed)
        except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
            results[mode] = {"status": "fail", "error": f"{type(error).__name__}: {error}",
                             "directory": str((output / mode).relative_to(native.ROOT))}
            native.write_json(output / f"{mode}.failure.json", results[mode])
            if not args.keep_going:
                break
        print(json.dumps({"mode": mode, **results[mode]}), flush=True)
    final_version, final_libraries = native.environment()
    if final_version != version or final_libraries != libraries:
        raise ValueError("authenticated target environment changed during capture")
    if evidence["version"] != version:
        raise ValueError("captured version evidence changed during execution")
    for path in libraries:
        if native.sha(path) != evidence["libraries"][path.relative_to(native.ROOT).as_posix()]:
            raise ValueError(f"native library changed during capture: {path}")
    if asm is not None and native.sha(asm) != args.asm_sha256:
        raise ValueError("frozen ASM artifact changed during capture")
    for path, key in ((native.JAVA, "java_executable_sha256"), (Path(javac), "javac_executable_sha256"),
                      (Path(sys.executable), "python_executable_sha256"), (Path(__file__), "driver_sha256"),
                      (Path(native.__file__), "native_helper_sha256"), (native.PREPARATION, "preparation_sha256")):
        if native.sha(path) != evidence[key]:
            raise ValueError(f"capture tool changed during execution: {path}")
    status = "pass" if len(results) == len(modes) and all(row["status"] == "pass" for row in results.values()) else "fail"
    native.write_json(output / "results.json", {"minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
                      "status": status, "selected_modes": modes, "captures": results,
                      "environment_sha256": native.sha(output / "environment.json")})
    native.write_json(output / "manifest.json", {"schema": "bcore.migration.catalog-batch.v1", "status": status,
                      "files_sha256": {path.relative_to(output).as_posix(): native.sha(path)
                                       for path in sorted(output.iterdir()) if path.is_file()},
                      "capture_manifests": {mode: row["manifest_sha256"] for mode, row in results.items()
                                            if row["status"] == "pass"}})
    print(json.dumps({"output": str(output), "status": status, "captures": results}, indent=2), flush=True)
    if status != "pass":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
