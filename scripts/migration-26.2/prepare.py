"""Preserve the current implementation and authenticate the official 26.2 server."""
from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "scripts/benchmarks"))
import provenance
import restore_native_environment as native


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists() or not output.is_relative_to(ROOT / "target"):
        parser.error("output must be a new directory under target")
    output.mkdir(parents=True)
    provenance.freeze_copy(__file__, output / "prepare.py")
    before = provenance.source_hashes(ROOT)
    _, snapshot = provenance.snapshot(ROOT, output, before)
    native.download(native.VERSION_MANIFEST, output / "version-manifest.json")
    manifest = json.loads((output / "version-manifest.json").read_bytes())
    entries = [row for row in manifest["versions"] if row["id"] == "26.2"]
    if len(entries) != 1:
        raise ValueError("official manifest does not contain one 26.2 release")
    entry = entries[0]
    metadata = output / "version-26.2.json"
    native.download(entry["url"], metadata, expected={"sha1": entry["sha1"]})
    version = json.loads(metadata.read_bytes())
    if version["id"] != "26.2" or version["type"] != "release":
        raise ValueError("target is not the official 26.2 release")
    server = version["downloads"]["server"]
    bundle = output / "server-26.2-bundle.jar"
    transfer = native.download(server["url"], bundle, expected={"sha1": server["sha1"]}, size=server["size"])
    installed = ROOT / "target/vanilla-776"
    files = {}
    with zipfile.ZipFile(bundle) as archive:
        versions = native.manifest_rows(archive.read("META-INF/versions.list"))
        libraries = native.manifest_rows(archive.read("META-INF/libraries.list"))
        if len(versions) != 1 or versions[0][1:] != ("26.2", "26.2/server-26.2.jar"):
            raise ValueError("official inner-server identity differs")
        for kind, rows in (("versions", versions), ("libraries", libraries)):
            for digest, _, name in rows:
                destination = installed / kind / native.member_path(name)
                files[destination.relative_to(ROOT).as_posix()] = native.install_member(
                    archive, f"META-INF/{kind}/{name}", destination, digest)
    jar = installed / "versions/26.2/server-26.2.jar"
    old_jar = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"
    with zipfile.ZipFile(jar) as target, zipfile.ZipFile(old_jar) as old:
        identity = json.loads(target.read("version.json"))
        if identity["name"] != "26.2" or identity["protocol_version"] != 776:
            raise ValueError("inner version/protocol identity differs")
        old_names, new_names = set(old.namelist()), set(target.namelist())
        changed = [name for name in sorted(old_names & new_names)
                   if not name.endswith("/") and old.read(name) != target.read(name)]
        resources = [name for name in changed if name.startswith("data/")]
        worldgen = [name for name in resources if name.startswith("data/minecraft/worldgen/")]
        difference = {"added": sorted(new_names - old_names), "removed": sorted(old_names - new_names),
                      "changed": changed, "changed_worldgen_resources": worldgen,
                      "changed_resource_categories": dict(Counter("/".join(name.split("/")[:3]) for name in resources))}
        provenance.write_json(output / "artifact-differences.json", difference)
        provenance.write_json(output / "inner-version.json", identity)
    result = {"schema": 1, "target": identity, "server_jar_sha256": provenance.sha(jar),
              "server_bundle": transfer, "version_entry": entry,
              "metadata_sha256": provenance.sha(metadata), "files_sha256": files,
              "baseline_snapshot": snapshot, "artifact_differences_sha256": provenance.sha(output / "artifact-differences.json")}
    provenance.write_json(output / "results.json", result)
    print(json.dumps({"output": str(output), "target": identity, "server_jar_sha256": result["server_jar_sha256"],
                      "libraries": len(libraries), "preserved_source_inputs": len(before),
                      "changed_worldgen_resources": worldgen,
                      "added_worldgen_resources": [name for name in difference["added"] if name.startswith("data/minecraft/worldgen/")]},
                     indent=2), flush=True)


if __name__ == "__main__":
    main()
