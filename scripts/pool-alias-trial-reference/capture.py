"""Owned native pool-alias/trial probes; reuses the shared jigsaw asset extractor.

The shared JigsawAssets launcher cannot initialize VaultConfig until native item
components are bound. Only the launcher is substituted for asset captures; the
unmodified jigsaw-reference/capture.py extract() performs template decoding,
JAR extraction, hashing and exclusive output creation. All launchers and shared
sources are included in provenance. Expected results come only from the JAR.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
SHARED_PATH = ROOT / "scripts/jigsaw-reference/capture.py"
SPEC = importlib.util.spec_from_file_location("pool_alias_shared_capture", SHARED_PATH)
SHARED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SHARED)


def run_probe(args, name: str, probe_args=()):
    sources = [ROOT / "scripts/TreeReference.java", ROOT / "scripts/NativeWorldgenRegistries.java",
               ROOT / "scripts/jigsaw-reference/JigsawSupport.java",
               ROOT / "scripts/jigsaw-reference/JigsawReference.java"]
    if name == "JigsawAssets":
        sources += [ROOT / "scripts/jigsaw-reference/JigsawAssets.java",
                    Path(__file__).with_name("PoolAliasTrialAssetsBootstrap.java")]
        launcher, marker = "PoolAliasTrialAssetsBootstrap", "JIGSAWASSETS="
    else:
        sources += [Path(__file__).with_name("PoolAliasTrialReference.java")]
        launcher, marker = "PoolAliasTrialReference", "POOLALIASTRIALREFERENCE="
        if name == "trial":
            sources += [Path(__file__).with_name("TrialChambersNativeReference.java")]
            launcher = "TrialChambersNativeReference"
    provenance_sources = [*sources, SHARED_PATH, Path(__file__)]
    header = SHARED.provenance(args.jar, provenance_sources)
    header["capture_launcher"] = launcher
    header["source_sha256"] = {path.relative_to(ROOT).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                               for path in provenance_sources}
    args.build = args.build.resolve()
    args.jar = args.jar.resolve()
    args.build.mkdir(parents=True, exist_ok=True)
    for path in provenance_sources:
        saved = args.build / "source-snapshot" / path.relative_to(ROOT)
        saved.parent.mkdir(parents=True, exist_ok=True)
        if saved.exists() and saved.read_bytes() != path.read_bytes():
            raise ValueError(f"immutable probe source snapshot differs: {saved}; use a fresh --build")
        if not saved.exists():
            with saved.open("xb") as stream:
                stream.write(path.read_bytes())
    subprocess.run([args.javac, "-d", str(args.build), *map(str, sources)], check=True)
    libraries = sorted((args.jar.parents[2] / "libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [args.build, args.jar, *libraries]))
    result = subprocess.run([str(args.java.resolve()), "-Xmx3G", "-cp", classpath,
                             launcher, str(args.jar), *probe_args], cwd=args.build,
                            capture_output=True, text=True, timeout=600)
    (args.build / f"{name}.stdout.log").write_text(result.stdout, encoding="utf-8")
    (args.build / f"{name}.stderr.log").write_text(result.stderr, encoding="utf-8")
    if result.returncode or "Serialization errors:" in result.stdout + result.stderr:
        raise RuntimeError(result.stdout + result.stderr)
    lines = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(lines) != 1:
        raise ValueError(f"expected one {marker}; see {args.build}")
    return header | json.loads(lines[0])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["assets", "aliases", "trial", "summary"])
    parser.add_argument("--jar", type=Path, default=ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar")
    parser.add_argument("--java", type=Path, default=ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe")
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--verify", type=Path)
    parser.add_argument("--family", action="append", choices=["trial_chambers", "ancient_city", "village", "trail_ruins"])
    args = parser.parse_args()
    if args.mode == "summary":
        data = json.loads(args.output.read_text(encoding="utf-8"))
        for key, value in data.items():
            if isinstance(value, (list, dict)):
                print(key, len(value))
            else:
                print(key, value)
        for case in data.get("cases", []):
            print(case["name"], case.get("codec_error", "accepted"), "samples", len(case.get("samples", [])))
            if case["name"] in ("trial_chambers", "fractional_weight", "oversized_weight", "empty_namespace_and_path"):
                print(json.dumps(case.get("encoded"), indent=2))
        for case in data.get("assembly", []):
            print("assembly", case["label"], case["seed"], case["chunk"], "admitted", case["admitted"],
                  "pieces", len(case["pieces"]), "RNG", case["next_i64"])
        for case in data.get("placement", []):
            kinds = {}
            for entity in case["block_entities"]:
                kind = entity["nbt"]["Compound"]["id"]["String"]
                kinds[kind] = kinds.get(kind, 0) + 1
            print("placement", case["seed"], case["terrain"], "entire", case["entire"], "status", case["status"],
                  "writes", case["write_count"], "states", case["state_count"], "BE", kinds, "error", case.get("error"))
        for case in data.get("block_entity_loads", []):
            print("BE load", case["template"], "state", case["state"], "saved", json.dumps(case["nbt"]))
        if "templates" in data:
            config = data["jar_assets"]["data/minecraft/worldgen/structure/trial_chambers.json"]["value"]
            print("trial configuration", json.dumps(config, indent=2))
            print("block-entity defaults", json.dumps(data["block_entities"], indent=2))
            for name, template in data["templates"].items():
                if template["entities"]:
                    print("entity-bearing", name, json.dumps(template["entities"]))
                if "/spawner/" in name or "/reward/" in name or "/chests/" in name:
                    nbt = [b[4] for b in template["blocks"] if len(b) == 5]
                    print("special template", name, json.dumps(nbt))
        return
    if args.output.exists():
        parser.error("native outputs are immutable; choose a new --output")
    if args.mode == "assets":
        if not args.family or "trial_chambers" not in args.family:
            parser.error("trial assets require --family trial_chambers")
        SHARED.run_probe = run_probe
        SHARED.extract(args)
    else:
        data = SHARED.decode_fixture(run_probe(args, args.mode, [args.mode]))
        SHARED.write_result(args, data)


if __name__ == "__main__":
    main()
