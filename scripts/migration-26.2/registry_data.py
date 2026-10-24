"""Project authenticated native 26.2 registry and worldgen inputs for publication."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import zipfile

import native

CORE = native.ROOT / "target/migration-26_2-core-20261021-02"
ORIGINAL = native.ROOT / "target/migration-26_2-registry-data-20261022-02"
PUBLISHED = {
    "generated.rs": "crates/bcore-registry/src/generated.rs",
    "generated_source_26_2.json": "crates/bcore-registry/data/generated_source_26_2.json",
    "registries_26_2.json": "crates/bcore-registry/data/registries_26_2.json",
    "biome_parameters_26_2.json": "crates/bcore-worldgen/data/biome_parameters_26_2.json",
    "vanilla_worldgen_26_2.json": "crates/bcore-worldgen/data/vanilla_worldgen_26_2.json",
}


def canonical(value) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
                      allow_nan=False).encode("utf-8")


def compact(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(canonical(value) + b"\n")


def state_identity(row: dict) -> tuple:
    return row["name"], tuple(sorted(row["properties"].items()))


def rust_identifier(name: str) -> str:
    value = re.sub(r"[^A-Za-z0-9_]", "_", name.removeprefix("minecraft:")).upper()
    if not value or value[0].isdigit():
        raise ValueError(f"invalid Rust registry identifier: {name}")
    return value


def old_states() -> dict[int, tuple]:
    catalog = json.loads((native.ROOT / "crates/bcore-worldgen/data/base_feature_catalog_26_1.json").read_bytes())
    result = {}
    for name, block in catalog["blocks"].items():
        for offset in range(block["count"]):
            index = offset
            properties = {}
            for key, values in reversed(block["properties"]):
                properties[key] = values[index % len(values)]
                index //= len(values)
            if index != 0:
                raise ValueError(f"old property cardinality differs: {name}")
            result[block["first"] + offset] = name, tuple(sorted(properties.items()))
    if set(result) != set(range(29873)):
        raise ValueError("historical state catalog is not complete")
    return result


def source_provenance(output: Path, fingerprint: str) -> dict:
    path = native.ROOT / "crates/bcore-registry/data/provenance_26_2.json"
    original = json.loads(path.read_bytes())
    if (original["minecraft"], original["protocol"], original["data_version"],
            original["jar_sha256"], original["registry_fingerprint"]) != (
            "26.2", 776, 4903, native.JAR_SHA256, fingerprint):
        raise ValueError("original registry provenance identity differs")
    if native.sha(CORE / "registries.json") != original["registry_capture_sha256"]:
        raise ValueError("original native registry capture differs")
    for name, expected in original["datagen_reports_sha256"].items():
        if native.sha(CORE / "datagen/reports" / name) != expected:
            raise ValueError(f"original datagen report differs: {name}")
    for name, expected in original["generated_sha256"].items():
        if name != "generated.rs" and native.sha(output / name) != expected:
            raise ValueError(f"regenerated registry data differs: {name}")
    old = ORIGINAL / "generated.rs"
    if native.sha(old) != original["generated_sha256"]["generated.rs"]:
        raise ValueError("preserved original generated source differs")
    before = old.read_text(encoding="utf-8").splitlines()[1:]
    after = (output / "generated.rs").read_text(encoding="utf-8").splitlines()[1:]
    if "".join("\n".join(before).split()) != "".join("\n".join(after).split()):
        raise ValueError("generated registry source changed beyond formatting and its provenance link")
    return {
        "schema": "bcore.registry-source.v1", "minecraft": "26.2", "protocol": 776,
        "data_version": 4903, "jar_sha256": native.JAR_SHA256,
        "registry_fingerprint": fingerprint,
        "registry_capture_sha256": native.sha(CORE / "registries.json"),
        "original_provenance_sha256": native.sha(path),
        "original_generated_sha256": original["generated_sha256"]["generated.rs"],
        "generator_sha256": native.sha(Path(__file__)),
        "generated_sha256": native.sha(output / "generated.rs"),
        "source_text_normalization": "utf-8-crlf-to-lf",
        "data_sha256": {name: digest for name, digest in original["generated_sha256"].items()
                        if name != "generated.rs"},
        "source_change": "rustfmt-layout-and-provenance-reference",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--verify-published", action="store_true",
                        help="verify canonical source text and byte-identical native data")
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists() or not output.is_relative_to(native.ROOT / "target"):
        parser.error("output must be a new directory under target/")
    version, _ = native.environment()
    core = json.loads((CORE / "registries.json").read_bytes())
    if core["jar_sha256"] != native.JAR_SHA256 or core["minecraft"] != version["name"]:
        raise ValueError("registry capture identity differs")
    states = core["states"]
    if [row["id"] for row in states] != list(range(32366)):
        raise ValueError("native state IDs are not complete")
    reports = CORE / "datagen/reports"
    blocks = json.loads((reports / "blocks.json").read_bytes())
    report_states = {}
    for name, definition in blocks.items():
        for row in definition["states"]:
            report_states[row["id"]] = (name, tuple(sorted(row.get("properties", {}).items())))
    if report_states != {row["id"]: state_identity(row) for row in states}:
        raise ValueError("independent native registry and datagen state identities differ")
    registries = core["registries"]
    for name, rows in registries.items():
        if [row["id"] for row in rows] != list(range(len(rows))):
            raise ValueError(f"registry IDs not contiguous: {name}")
    identity = {"minecraft": version["name"], "protocol": version["protocol_version"],
                "data_version": version["world_version"], "jar_sha256": native.JAR_SHA256,
                "registries": registries, "states": states}
    fingerprint = hashlib.sha256(canonical(identity)).digest()
    output.mkdir()
    compact(output / "registries_26_2.json", identity)
    compact(output / "biome_parameters_26_2.json", {
        "minecraft": "26.2", "protocol": 776, "jar_sha256": native.JAR_SHA256,
        "probe_sha256": core["probe_sha256"], "rows": core["climate_rows"]})
    assets = {}
    with zipfile.ZipFile(native.JAR) as archive:
        for name in sorted(archive.namelist()):
            prefix = "data/minecraft/worldgen/"
            if name.startswith(prefix) and name.endswith(".json"):
                relative = name[len(prefix):]
                if relative.split("/", 1)[0] in {"density_function", "noise", "noise_settings"}:
                    assets[relative] = json.loads(archive.read(name))
    assets["biome_registry.json"] = [row["key"] for row in registries["BIOME"]]
    assets["biome_parameters/overworld.json"] = core["parameters"]
    assets["worldgen_version.json"] = {"minecraft": "26.2", "protocol": 776,
                                     "data_version": 4903, "jar_sha256": native.JAR_SHA256,
                                     "climate_probe_sha256": core["probe_sha256"]}
    compact(output / "vanilla_worldgen_26_2.json", assets)
    targets = {state_identity(row): row["id"] for row in states}
    remapping = {str(id): targets.get(key) for id, key in old_states().items()}
    compact(output / "state-mapping-26_1-to-26_2.json", {
        "authority": "resource-name-and-complete-state-properties", "mapping": remapping,
        "unmapped": [int(id) for id, target in remapping.items() if target is None]})
    constants = {}
    prepared = json.loads(native.PREPARATION.read_bytes())
    source_path = native.PREPARATION.parent / "source/crates/bcore-worldgen/src/lib.rs"
    if native.sha(source_path) != prepared["baseline_snapshot"]["source_sha256"]["crates/bcore-worldgen/src/lib.rs"]:
        raise ValueError("preserved pre-migration constant source differs")
    source = source_path.read_text(encoding="utf-8")
    block_source = source.split("pub mod block {", 1)[1].split("\n}", 1)[0]
    for name, id in re.findall(r"pub const (\w+): u32 = (\d+);", block_source):
        value = remapping[id]
        if value is None:
            raise ValueError(f"unmapped current block constant {name}")
        constants[name] = value
    for row in states:
        if row["default"]:
            constants.setdefault(rust_identifier(row["name"]), row["id"])
    constants["CAVE_AIR"] = targets[("minecraft:cave_air", ())]
    constants["VOID_AIR"] = targets[("minecraft:void_air", ())]
    lines = ["// Generated from the authenticated 26.2 native registry; see data/generated_source_26_2.json.",
             "pub use bcore_core::version::{DATA_VERSION, MC_VERSION, PROTOCOL_VERSION};",
             'pub const SERVER_JAR_SHA256: &str =',
             f'    "{native.JAR_SHA256}";',
             f"pub const BLOCK_STATE_COUNT: usize = {len(states)};",
             f"pub const BIOME_COUNT: usize = {len(registries['BIOME'])};",
             "pub const REGISTRY_FINGERPRINT: [u8; 32] = [",]
    for offset in range(0, 32, 16):
        lines.append("    " + ", ".join(f"0x{byte:02x}" for byte in fingerprint[offset:offset + 16]) + ",")
    lines.extend(["];", "", "pub mod block_state {"])
    lines.extend(f"    pub const {name}: u32 = {value};" for name, value in sorted(constants.items()))
    for module, key in (("entity_type", "ENTITY_TYPE"), ("attribute", "ATTRIBUTE"),
                        ("item", "ITEM"), ("block_entity_type", "BLOCK_ENTITY_TYPE"),
                        ("component", "DATA_COMPONENT_TYPE"), ("biome", "BIOME")):
        lines.extend(["}", "", f"pub mod {module} {{"])
        identifiers = [rust_identifier(row["key"]) for row in registries[key]]
        if len(set(identifiers)) != len(identifiers):
            raise ValueError(f"Rust identifier collision in {key}")
        lines.extend(f"    pub const {name}: u32 = {row['id']};"
                     for name, row in zip(identifiers, registries[key], strict=True))
    lines.extend(["}", ""])
    with (output / "generated.rs").open("x", encoding="utf-8", newline="\n") as stream:
        stream.write("\n".join(lines))
    compact(output / "generated_source_26_2.json", source_provenance(output, fingerprint.hex()))
    verification = []
    if args.verify_published:
        for name, destination in PUBLISHED.items():
            generated = (output / name).read_bytes()
            path = native.ROOT / destination
            published = path.read_bytes()
            source_text = name in ("generated.rs", "generated_source_26_2.json")
            comparable = published.replace(b"\r\n", b"\n") if source_text else published
            if generated != comparable:
                raise ValueError(f"published regeneration differs: {destination}")
            verification.append({
                "destination": destination, "sha256": hashlib.sha256(published).hexdigest(),
                "generated_sha256": hashlib.sha256(generated).hexdigest(),
                "normalization": "utf-8-crlf-to-lf" if source_text else "none",
            })
        compact(output / "published_verification.json", {"files": verification})
    compact(output / "provenance_26_2.json", {
        "schema": 1, "minecraft": "26.2", "protocol": 776, "data_version": 4903,
        "jar_sha256": native.JAR_SHA256, "registry_fingerprint": fingerprint.hex(),
        "registry_capture_sha256": native.sha(CORE / "registries.json"),
        "datagen_reports_sha256": {p.name: native.sha(p) for p in reports.iterdir() if p.is_file()},
        "generator_sha256": native.sha(Path(__file__)),
        "generated_sha256": {p.name: native.sha(p) for p in output.iterdir() if p.is_file()}})
    print(json.dumps({"output": str(output), "registry_fingerprint": fingerprint.hex(),
                      "verified_published_file_count": len(PUBLISHED) if args.verify_published else 0,
                      "state_count": len(states), "biome_count": len(registries["BIOME"]),
                      "constant_changes": {name: constants[name] for name, _ in re.findall(
                          r"pub const (\w+): u32 = (\d+);", block_source)},
                      "unmapped_old_states": [int(id) for id, value in remapping.items() if value is None]},
                     indent=2), flush=True)


if __name__ == "__main__":
    main()
