"""Publish new authenticated 26.2 registry/catalog files without replacing historical inputs."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil

import native

REGISTRIES = native.ROOT / "target/migration-26_2-registry-data-20261022-02"
CATALOGS = native.ROOT / "target/migration-26_2-catalogs-20261021-01"
SURFACE = native.ROOT / "target/migration-26_2-surface-catalog-20261022-02"


def publish(source: Path, destination: Path, expected: str) -> dict:
    if native.sha(source) != expected:
        raise ValueError(f"authenticated generated input changed: {source}")
    if destination.exists():
        if native.sha(destination) != expected:
            raise ValueError(f"existing published file differs: {destination}")
    else:
        destination.parent.mkdir(parents=True, exist_ok=True)
        with source.open("rb") as reader, destination.open("xb") as writer:
            shutil.copyfileobj(reader, writer)
    if native.sha(destination) != expected:
        raise ValueError(f"published digest differs: {destination}")
    return {"source": source.relative_to(native.ROOT).as_posix(),
            "destination": destination.relative_to(native.ROOT).as_posix(), "sha256": expected}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists() or not output.is_relative_to(native.ROOT / "target"):
        parser.error("output must be a new directory under target/")
    native.environment()
    registry_manifest = json.loads((REGISTRIES / "provenance_26_2.json").read_bytes())
    if registry_manifest["jar_sha256"] != native.JAR_SHA256:
        raise ValueError("registry manifest version differs")
    files = []
    registry_destinations = {
        "generated.rs": "crates/bcore-registry/src/generated.rs",
        "registries_26_2.json": "crates/bcore-registry/data/registries_26_2.json",
        "biome_parameters_26_2.json": "crates/bcore-worldgen/data/biome_parameters_26_2.json",
        "vanilla_worldgen_26_2.json": "crates/bcore-worldgen/data/vanilla_worldgen_26_2.json",
    }
    for name, destination in registry_destinations.items():
        files.append(publish(REGISTRIES / name, native.ROOT / destination,
                             registry_manifest["generated_sha256"][name]))
    files.append(publish(REGISTRIES / "provenance_26_2.json",
                         native.ROOT / "crates/bcore-registry/data/provenance_26_2.json",
                         native.sha(REGISTRIES / "provenance_26_2.json")))
    for batch in (CATALOGS, SURFACE):
        result = json.loads((batch / "results.json").read_bytes())
        if result["jar_sha256"] != native.JAR_SHA256:
            raise ValueError("catalog batch version differs")
        for mode, row in result["captures"].items():
            if row["status"] != "pass":
                continue
            directory = native.ROOT / row["directory"]
            manifest_path = directory / "manifest.json"
            if native.sha(manifest_path) != row["manifest_sha256"]:
                raise ValueError(f"catalog manifest changed: {mode}")
            manifest = json.loads(manifest_path.read_bytes())
            if manifest["jar_sha256"] != native.JAR_SHA256 or manifest["catalog_sha256"] != row["catalog_sha256"]:
                raise ValueError(f"catalog identity differs: {mode}")
            crate = "bcore-protocol" if mode == "spawn_handoff" else "bcore-worldgen"
            destination = native.ROOT / f"crates/{crate}/data/{row['catalog']}"
            files.append(publish(directory / row["catalog"], destination, row["catalog_sha256"]))
    output.mkdir()
    native.write_json(output / "results.json", {"minecraft": "26.2", "protocol": 776,
                      "jar_sha256": native.JAR_SHA256, "published": files,
                      "generator_sha256": native.sha(Path(__file__))})
    print(json.dumps({"output": str(output), "published_file_count": len(files)}, indent=2), flush=True)


if __name__ == "__main__":
    main()
