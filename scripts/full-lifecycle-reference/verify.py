"""Check the captured FULL boundaries and package their native provenance."""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_capture(directory):
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    for name, expected in manifest.items():
        if sha(directory / name) != expected:
            raise ValueError(f"capture file changed: {directory / name}")
    data = json.loads((directory / "observations.json").read_text(encoding="utf-8"))
    if data["jar_sha256"] != JAR_SHA256 or data["gameplay_ticks"] != 0:
        raise ValueError("wrong native authority or a gameplay tick occurred")
    provenance = json.loads((directory / "provenance.json").read_text(encoding="utf-8"))
    for name, expected in provenance["sources"].items():
        if sha(directory / "build" / Path(name).name) != expected:
            raise ValueError(f"frozen source changed: {name}")
    if sha(directory / "build/capture.py") != provenance["runner_sha256"]:
        raise ValueError("frozen capture runner changed")
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("first", type=Path)
    parser.add_argument("repeat", type=Path)
    parser.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    first, repeat = read_capture(args.first), read_capture(args.repeat)
    if first != repeat:
        raise ValueError("independent native observations differ")
    provenance = [json.loads((directory / "provenance.json").read_text(encoding="utf-8"))
                  for directory in (args.first, args.repeat)]
    for key in ("jar_sha256", "asm_sha256", "sources", "runner_sha256", "server_properties", "java", "javac", "libraries"):
        if provenance[0][key] != provenance[1][key]:
            raise ValueError(f"independent capture inputs differ: {key}")
    rows = {row["boundary"]: row for row in first["observations"]}
    proto, full, saved = (rows[key] for key in (
        "before FULL", "after FULL before serialization", "after level serialization"))
    after_repeat, unpacked = rows["after repeat FULL"], rows["after unpackTicks"]
    holder, lower = rows["holder after FULL"], rows["lower-status access after FULL"]
    checks = {
        "full_constructs_level_chunk": proto["class"] == "ProtoChunk" and full["class"] == "LevelChunk" and full["persisted"] == "minecraft:full",
        "full_keeps_pending_until_save": len(proto["pending"]) == 2 and full["pending"] == proto["pending"] and full["loaded"] == proto["loaded"],
        "level_save_materializes_pending": not saved["pending"] and len(saved["loaded"]) == 3,
        "constructor_filters_only_wg_maps": set(proto["heightmaps"]) - set(full["heightmaps"]) == {"WORLD_SURFACE_WG", "OCEAN_FLOOR_WG"}
            and rows["old proto retained after FULL"]["heightmaps"] == proto["heightmaps"],
        "full_and_save_keep_marks": sum(map(len, proto["marks"])) == 2 and proto["marks"] == full["marks"] == saved["marks"] == unpacked["marks"],
        "full_registers_both_tick_containers": not proto["block_ticks_registered"] and not proto["fluid_ticks_registered"]
            and full["block_ticks_registered"] and full["fluid_ticks_registered"],
        "full_carries_deduplicated_saved_ticks": proto["block_ticks_count"] == full["block_ticks_count"] == 2
            and proto["fluid_ticks_count"] == full["fluid_ticks_count"] == 1
            and full["block_ticks_class"] == full["fluid_ticks_class"] == "LevelChunkTicks" and full["block_tick_head"] is None,
        "holder_replacement_shares_sections_and_live_entity": holder["lower_status_class"] == "ImposterProtoChunk"
            and holder["wrapped_is_full"] and holder["sections_shared"] and holder["loaded_entity_shared"],
        "lower_status_writes_blocked_and_wg_query_redirected": lower["write_returned_null"] and lower["write_left_air"]
            and lower["wg_height"] == lower["runtime_height"] == 80,
        "repeat_preserves_full_identity_and_effects": rows["repeat FULL"]["same_chunk"]
            and {k: v for k, v in after_repeat.items() if k != "boundary"} == {k: v for k, v in saved.items() if k != "boundary"},
        "unpack_activates_saved_ticks_without_execution": unpacked["block_ticks_count"] == 2
            and unpacked["block_tick_head"] == {"type": "Block{minecraft:stone}", "trigger_tick": 0, "priority": "HIGH", "sub_tick_order": -1},
    }
    failed = [name for name, passed in checks.items() if not passed]
    if failed:
        raise ValueError(f"unexpected native boundary observations: {failed}")
    report = {"jar_sha256": JAR_SHA256, "observations_sha256": sha(args.first / "observations.json"),
              "verifier_sha256": sha(Path(__file__)), "gameplay_ticks": first["gameplay_ticks"],
              "checks": checks, "independent_repeat_matches": True}
    with zipfile.ZipFile(args.archive, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("verification.json", json.dumps(report, indent=2) + "\n")
        archive.write(Path(__file__), "verify.py")
        for label, directory in (("first", args.first), ("repeat", args.repeat)):
            for path in sorted(directory.iterdir()):
                if path.is_file():
                    archive.write(path, f"{label}/{path.name}")
            for path in sorted((directory / "build").iterdir()):
                if path.suffix in (".java", ".py"):
                    archive.write(path, f"{label}/build/{path.name}")
    print(json.dumps({**report, "archive": str(args.archive), "archive_sha256": sha(args.archive)}, indent=2))


if __name__ == "__main__":
    main()
