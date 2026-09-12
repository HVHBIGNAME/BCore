"""Capture coordinate-addressed vanilla data and compare freshly built BCore chunks.

The capture can be reused while fixing worldgen; it never depends on grid axis order.
Run from the repository root after building the dump_chunk release example.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from parity_harness import IDS, LOGS, SEED, SAMPLES


def run(command, cwd=ROOT):
    result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, timeout=180)
    if result.returncode:
        raise RuntimeError(f"{command}: {result.stderr}")
    return result


def compare(capture, executable, seed):
    chunks = {}
    diffs = Counter()
    exact_states = 0
    vanilla_origins, bcore_origins = set(), set()
    terrain_matches = 0
    height_deltas = Counter()
    biome_matches = 0
    vanilla_blocks = {tuple(row[:3]): row[3] for row in capture["blocks"]}
    bcore_blocks = {}
    for x, y, z, name, state in capture["blocks"]:
        key = (x // 16, z // 16)
        if key not in chunks:
            raw = json.loads(run([str(executable), str(seed), *map(str, key)]).stdout)
            if len(raw["states"]) != 384 * 256:
                raise ValueError("incomplete BCore chunk")
            chunks[key] = raw
        actual = chunks[key]["states"][(y + 64) * 256 + (z % 16) * 16 + x % 16]
        actual_name = IDS.get(actual, f"state_{actual}")
        bcore_blocks[(x, y, z)] = actual_name
        exact_states += actual == state
        if name != actual_name:
            diffs[(name, actual_name)] += 1
    for x, z, top, terrain in capture["columns"]:
        raw = chunks[(x // 16, z // 16)]
        if "heights" in raw and terrain is not None:
            delta = raw["heights"][(z % 16) * 16 + x % 16] - terrain
            terrain_matches += delta == 0
            height_deltas[delta] += 1
    for blocks, origins in [(vanilla_blocks, vanilla_origins), (bcore_blocks, bcore_origins)]:
        for (x, y, z), name in blocks.items():
            if name in LOGS and blocks.get((x, y - 1, z)) not in LOGS:
                origins.add((x, y, z))
    for x, y, z, name in capture["biomes"]:
        raw = chunks[(x // 16, z // 16)]
        if "biomes" in raw:
            index = ((y + 64) // 4) * 16 + ((z % 16) // 4) * 4 + (x % 16) // 4
            biome_matches += raw["biomes"][index] == name
    return {
        "center": capture["center"], "cells": len(vanilla_blocks),
        "equal_states": exact_states, "different_names": sum(diffs.values()),
        "columns": len(capture["columns"]), "terrain_matches": terrain_matches,
        "height_deltas": dict(sorted(height_deltas.items())),
        "vanilla_tree_origins": len(vanilla_origins), "bcore_tree_origins": len(bcore_origins),
        "tree_intersection": len(vanilla_origins & bcore_origins),
        "vanilla_biomes": dict(Counter(row[3] for row in capture["biomes"])),
        "biome_cells": len(capture["biomes"]), "biome_matches": biome_matches,
        "top_differences": [[a, b, n] for (a, b), n in diffs.most_common(12)],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true", help="query vanilla instead of reusing saved captures")
    parser.add_argument("--directory", type=Path, default=ROOT / "target" / "parity-snapshots")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--reference", type=Path, help="export captured heights/biomes for offline Rust regression tests")
    args = parser.parse_args()
    if not args.output and not args.reference:
        parser.error("provide --output or --reference")
    executable = ROOT / "target" / "release" / "examples" / ("dump_chunk.exe" if sys.platform == "win32" else "dump_chunk")
    args.directory.mkdir(parents=True, exist_ok=True)
    if args.capture:
        seed_probe = ROOT / "scripts" / "bot" / "query_seed.js"
        actual_seed = int(run(["node", str(seed_probe)], seed_probe.parent).stdout.strip())
        if actual_seed != SEED:
            raise ValueError(f"vanilla seed {actual_seed} differs from expected {SEED}")
    records = []
    references = []
    for x, z in SAMPLES:
        path = args.directory / f"vanilla-{SEED}-{x}-{z}.json"
        if args.capture:
            bot = ROOT / "scripts" / "bot" / "dump_terrain.js"
            result = run(["node", str(bot), "127.0.0.1", "25571", str(x), str(z), "16", "bot", "--full", "--ymin=-64", "--ymax=319"], bot.parent)
            if "position after teleport" not in result.stderr:
                raise ValueError("vanilla teleport was not verified")
            capture = json.loads(result.stdout)
            capture["seed"] = SEED
            path.write_text(json.dumps(capture), encoding="utf-8")
        capture = json.loads(path.read_text(encoding="utf-8"))
        if capture["seed"] != SEED or len(capture["blocks"]) != 16 * 16 * 384:
            raise ValueError(f"invalid capture {path}")
        references.append({"center": capture["center"], "columns": capture["columns"], "biomes": capture["biomes"],
                           "capture_sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
        if not args.output:
            continue
        record = compare(capture, executable, SEED)
        record["capture_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        records.append(record)
        print(json.dumps(record), flush=True)
    report = {"seed": SEED, "protocol": 775, "ymin": -64, "ymax": 319,
              "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
              "samples": records}
    if args.output:
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if args.reference:
        args.reference.write_text(json.dumps({"seed": SEED, "minecraft": "26.1", "protocol": 775, "samples": references}, separators=(",", ":")) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
