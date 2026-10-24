"""Promote SHA-256-pinned 26.2 native loot exports into versioned runtime data."""
from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import argparse
import json
from pathlib import Path

import native
import loot_catalog_data as data


def check_worktree(worktree: Path) -> None:
    data.require(worktree == native.ROOT or worktree.is_relative_to(native.ROOT / "target")
                 and (worktree / ".git").is_file(), "destination must be this repository or a linked worktree under target/")
    data.require((worktree / ".git").exists() and (worktree / "crates/bcore-worldgen/data").is_dir(),
                 "destination worktree is not initialized")


def preflight(files: dict[Path, bytes]) -> None:
    for destination, raw in files.items():
        if destination.exists():
            data.require(destination.is_file() and destination.read_bytes() == raw,
                         f"refusing to replace different versioned loot data: {destination}")


def publish(files: dict[Path, bytes]) -> None:
    preflight(files)
    for destination, raw in files.items():
        if not destination.exists():
            destination.parent.mkdir(parents=True, exist_ok=True)
            with destination.open("xb") as stream:
                stream.write(raw)
        data.require(destination.read_bytes() == raw, f"promotion readback mismatch: {destination}")


def existing_runtime_hashes(runtime: Path, worktree: Path, captured: dict) -> dict[str, str]:
    promotion = data.load(runtime / "promotion.json")
    data.require(promotion["minecraft"] == "26.2" and promotion["server_sha256"] == native.JAR_SHA256,
                 "existing runtime promotion has a different native identity")
    hashes = {}
    for name in data.ENGINE_FILES:
        destination = runtime / name
        raw = destination.read_bytes()
        key = destination.relative_to(worktree).as_posix()
        hashes[name] = data.digest(raw)
        data.require(hashes[name] == promotion["files"][key]["sha256"]
                     and data.decode(raw) == data.decode(captured[name]),
                     f"fill fixture and promoted runtime context differ: {name}")
    return hashes


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--manifest-sha256", required=True, help="exact digest printed by loot_catalog.py")
    parser.add_argument("--worktree", type=Path, default=native.ROOT)
    parser.add_argument("--fixture-only", action="store_true", help="add a checked fill fixture against the already promoted runtime context")
    parser.add_argument("--inspect", action="store_true", help="validate and print the promotion without publishing files")
    args = parser.parse_args()
    source, worktree = args.source.resolve(), args.worktree.resolve()
    check_worktree(worktree)
    manifest, exports = data.verified_capture(source, args.manifest_sha256)
    version, libraries = native.environment()
    environment = data.load(source / "environment.json")
    data.require(environment["version"] == version and environment["libraries"] == {
        path.relative_to(native.ROOT).as_posix(): native.sha(path) for path in libraries},
        "capture and authenticated target environments disagree")
    captured = {name: (source / name).read_bytes() for name in data.RUNTIME_FILES}
    if manifest["mode"] == "fill":
        captured["loot-fill-26.2.json"] = (source / "loot-fill-26.2.json").read_bytes()
    for name, raw in captured.items():
        data.require(data.digest(raw) == manifest["files_sha256"][name], f"native export changed while preparing promotion: {name}")
    runtime = worktree / "crates/bcore-worldgen/data/loot_26_2"
    runtime_hashes = {name: data.digest(captured[name]) for name in data.ENGINE_FILES}
    if args.fixture_only:
        data.require(manifest["mode"] == "fill", "--fixture-only requires a native fill capture")
        runtime_hashes = existing_runtime_hashes(runtime, worktree, captured)
        files = {}
    else:
        files = {runtime / name: captured[name] for name in data.RUNTIME_FILES}
    if manifest["mode"] == "fill":
        fixture_directory = worktree / "crates/bcore-worldgen/tests/data/loot_26_2"
        files[fixture_directory / "loot-fill-26.2.json"] = captured["loot-fill-26.2.json"]
        fixture_provenance = {"schema": "bcore.loot-fixture-promotion.v1", "minecraft": "26.2", "protocol": 776,
                              "server_sha256": native.JAR_SHA256, "capture_manifest_sha256": args.manifest_sha256,
                              "fixture_sha256": data.digest(captured["loot-fill-26.2.json"]),
                              "native_case_count": manifest["case_count"], "independent_repeat": manifest["independent_repeat"],
                              "runtime_sha256": runtime_hashes,
                              "captured_runtime_sha256": {name: data.digest(captured[name]) for name in data.ENGINE_FILES},
                              "context_comparison": "Exact JSON values; object key order is not a value, native arrays and holder order are retained."}
        files[fixture_directory / "promotion.json"] = (
            json.dumps(fixture_provenance, indent=2, sort_keys=True, allow_nan=False) + "\n").encode("utf-8")
    outputs = {path.relative_to(worktree).as_posix(): {
        "source": path.name, "bytes": len(raw), "sha256": data.digest(raw)} for path, raw in files.items()}
    provenance = {"schema": "bcore.loot-data-promotion.v2", "minecraft": "26.2", "protocol": 776,
                  "data_version": 4903, "server_sha256": native.JAR_SHA256,
                  "capture_manifest_sha256": args.manifest_sha256, "capture": str(source),
                  "mode": manifest["mode"], "counts": exports["loot-catalog.json"]["counts"],
                  "native_case_count": manifest["case_count"], "independent_repeat": manifest["independent_repeat"],
                  "promoter_sha256": native.sha(Path(__file__)),
                  "validator_sha256": native.sha(Path(data.__file__)), "files": outputs}
    if not args.fixture_only:
        files[runtime / "promotion.json"] = (
            json.dumps(provenance, indent=2, sort_keys=True, allow_nan=False) + "\n").encode("utf-8")
    preflight(files)
    if not args.inspect:
        data.verified_capture(source, args.manifest_sha256)
        if args.fixture_only:
            data.require(existing_runtime_hashes(runtime, worktree, captured) == runtime_hashes,
                         "promoted runtime changed during fixture admission")
        publish(files)
    print(json.dumps({"status": "pass", "published": not args.inspect, **provenance}, indent=2), flush=True)


if __name__ == "__main__":
    main()
