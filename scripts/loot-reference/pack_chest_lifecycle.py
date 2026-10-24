"""Package verified independent chest lifecycle observations without rewriting them."""

import argparse
import base64
import importlib.util
import os
from pathlib import Path
import zipfile

from chest_lifecycle import PRIOR, PRIOR_FIXTURE_SHA256, load, new_output, require, sha
from chest_lifecycle_verify import canonical
from inventory import JAR, JAR_SHA256, ROOT, digest, write_json


def copy(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("xb") as stream:
        stream.write(source.read_bytes())
    require(sha(source) == sha(destination), "packaged file readback differs")


def verify_manifest(directory):
    manifest = load(directory / "manifest.json")
    for path, expected in manifest.items():
        require(sha(directory / path) == expected, f"immutable capture changed: {directory / path}")
    return len(manifest)


def source_jobs(directories):
    jobs = []
    for directory in directories:
        matches = list((ROOT / "target/generation-checks").glob(directory.name + "-*.json"))
        require(len(matches) == 1, f"ambiguous or absent capture job: {directory.name}")
        record = load(matches[0])
        require(record["state"] == "finished" and record.get("exit_code") == 0, f"capture job did not pass: {record['job']}")
        require(record["cargo_target_dir"] == str(ROOT / "target/build"), "capture did not use the shared build queue")
        jobs.append(record)
    return jobs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--first", type=Path, required=True)
    parser.add_argument("--repeat", type=Path, required=True)
    parser.add_argument("--inspect", type=Path, required=True)
    parser.add_argument("--bytecode", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    require(os.environ.get("CARGO_TARGET_DIR") == str(ROOT / "target/build") and os.environ.get("CARGO_BUILD_JOBS") == "1",
            "use the ROOT serialized heavy-check queue")
    verified_files = {path.name: verify_manifest(path) for path in (args.first, args.repeat, args.inspect)}
    first, repeat = load(args.first / "provenance.json"), load(args.repeat / "provenance.json")
    for field in ("jar_sha256", "java", "javac", "java_executable_sha256", "javac_executable_sha256", "sources", "nbt_decoder_sha256",
                  "inputs_sha256", "prior_fixture_sha256", "compiled_classes", "libraries", "server_properties"):
        require(first[field] == repeat[field], f"independent native authority differs: {field}")
    require(first["jar_sha256"] == sha(JAR) == JAR_SHA256, "native JAR identity differs")
    require(first["prior_fixture_sha256"] == sha(PRIOR / "loot-fill-26.1.json") == PRIOR_FIXTURE_SHA256, "prior artifact identity differs")
    fixture_name = "chest-lifecycle-26.1.json"
    require((args.first / fixture_name).read_bytes() == (args.repeat / fixture_name).read_bytes(), "compact lifecycle repeats differ byte-for-byte")
    raw_first, raw_repeat = args.first / "observations.raw.json", args.repeat / "observations.raw.json"
    raw_canonical = canonical(load(raw_first))
    require(raw_canonical == canonical(load(raw_repeat)), "raw lifecycle values or array order differ")
    for directory in (args.first, args.repeat):
        verification = load(directory / "verification.json")
        require(verification["status"] == "pass" and verification["fixture_sha256"] == sha(directory / fixture_name),
                "capture verification or fixture identity differs")
    require(load(args.repeat / "verification.json")["independent_repeat_equal"], "repeat was not verified at capture time")
    jobs = source_jobs((args.first, args.repeat, args.inspect, *args.bytecode))
    out = new_output(args.output)
    copy(args.first / fixture_name, out / fixture_name)
    fixture = load(out / fixture_name)
    codec_path = args.first / "build/nbt_reader_reference.py"
    module_spec = importlib.util.spec_from_file_location("chest_lifecycle_pack_nbt", codec_path)
    codec = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(codec)
    nbt_dir = out / "nbt"
    nbt_dir.mkdir()
    for key, blob in fixture["nbt_blobs"].items():
        raw = base64.b64decode(blob["base64"], validate=True)
        require(digest(raw) == key and codec.NbtReader(raw).root() == blob["typed"], "native NBT type/hash mismatch")
        with (nbt_dir / (key + ".nbt")).open("xb") as stream:
            stream.write(raw)
        require(sha(nbt_dir / (key + ".nbt")) == key, "native NBT readback mismatch")
    cases = []
    for case in fixture["cases"]:
        snapshots = {}
        for boundary, snapshot in case["snapshots"].items():
            root = fixture["nbt_blobs"][snapshot["saved_nbt_sha256"]]["typed"]["Compound"]
            snapshots[boundary] = {"loot_table": snapshot["loot_table"], "loot_seed_i64": snapshot["loot_seed_i64"],
                "saved_nbt_sha256": snapshot["saved_nbt_sha256"], "saved_tag_types": {name: next(iter(value)) for name, value in root.items()},
                "occupied_slots": [{"slot": slot, **item} for slot, item in enumerate(snapshot["slots"]) if item]}
        cases.append({"id": case["id"], "input_nbt_sha256": case["input_nbt_sha256"], "snapshots": snapshots})
    write_json(out / "case-index.json", {"cases": cases})
    native_classes = []
    with zipfile.ZipFile(JAR) as original:
        for directory in args.bytecode:
            metadata = load(directory / "provenance.json")
            require(metadata["jar_sha256"] == JAR_SHA256, "disassembly is not pinned 26.1")
            for item in metadata["classes"]:
                require(item["exit_code"] == 0 and sha(directory / item["output"]) == item["output_sha256"], "native bytecode listing differs")
                require(digest(original.read(item["member"])) == item["class_sha256"], "native class differs from the pinned JAR")
                native_classes.append({"class": item["name"], "sha256": item["class_sha256"],
                    "listing": directory.name + "/" + item["output"], "listing_sha256": item["output_sha256"]})
    write_json(out / "native-classes.json", native_classes)
    write_json(out / "source-jobs.json", jobs)
    copy(args.inspect / "reused-authority.json", out / "reused-authority.json")
    copy(args.first / "verification.json", out / "first-verification.json")
    copy(args.repeat / "verification.json", out / "repeat-verification.json")
    result = {"status": "pass", "fixture_sha256": sha(out / fixture_name), "fixture_bytes": (out / fixture_name).stat().st_size,
        "case_count": len(fixture["cases"]), "nbt_blob_count": len(fixture["nbt_blobs"]),
        "independent_repeat": {"compact_bytes_equal": True, "raw_bytes_equal": raw_first.read_bytes() == raw_repeat.read_bytes(),
            "raw_json_values_equal": True, "raw_comparison": "Canonical JSON; sorted object keys, preserved values and array order",
            "raw_canonical_sha256": digest(raw_canonical.encode("utf-8"))},
        "first_capture": str(args.first.resolve()), "repeat_capture": str(args.repeat.resolve()), "verified_manifest_files": verified_files,
        "raw_sha256": {"first": sha(raw_first), "repeat": sha(raw_repeat)}, "first_provenance_sha256": sha(args.first / "provenance.json"),
        "repeat_provenance_sha256": sha(args.repeat / "provenance.json"), "raw_inputs_sha256": first["inputs_sha256"],
        "prior_fixture_sha256": PRIOR_FIXTURE_SHA256,
        "static_only": ["Real player/advancement/menu gating", "Slot missing/error defaults and unsigned slots beyond the three preset positions"],
        "not_exercised": ["Real player menu", "Seed0 loot fill or sequence fallback", "Whole-server restart or region-file/chunk reload"]}
    write_json(out / "acceptance.json", result)
    for name in ("pack_chest_lifecycle.py", "chest_lifecycle.py", "chest_lifecycle_verify.py", "inventory.py"):
        copy(Path(__file__).parent / name, out / "research-scripts" / name)
    with zipfile.ZipFile(out / "evidence.zip", "x", zipfile.ZIP_DEFLATED) as archive:
        for directory in (args.first, args.repeat, args.inspect, *args.bytecode):
            for path in sorted(directory.rglob("*")):
                if not path.is_file():
                    continue
                relative = path.relative_to(directory)
                archive.write(path, directory.name + "/" + relative.as_posix())
        for path in sorted(out.rglob("*")):
            if path.is_file() and path.name != "evidence.zip":
                archive.write(path, "delivery/" + path.relative_to(out).as_posix())
    with zipfile.ZipFile(out / "evidence.zip") as archive:
        require(archive.testzip() is None, "evidence ZIP failed CRC validation")
        require(archive.read("delivery/" + fixture_name) == (out / fixture_name).read_bytes(), "ZIP fixture readback differs")
        for directory in (args.first, args.repeat, args.inspect):
            for relative, expected in load(directory / "manifest.json").items():
                require(digest(archive.read(directory.name + "/" + relative)) == expected, "ZIP capture manifest readback differs")
        for item in native_classes:
            require(digest(archive.read(item["listing"])) == item["listing_sha256"], "ZIP bytecode readback differs")
    write_json(out / "manifest.json", {path.relative_to(out).as_posix(): sha(path) for path in sorted(out.rglob("*")) if path.is_file()})
    print({"output": str(out), "fixture_sha256": result["fixture_sha256"], "fixture_bytes": result["fixture_bytes"],
           "evidence_sha256": sha(out / "evidence.zip"), "cases": len(fixture["cases"]), "status": "pass"})


if __name__ == "__main__":
    main()
