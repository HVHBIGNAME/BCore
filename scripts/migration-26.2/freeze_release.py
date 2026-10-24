"""Freeze a successfully built 26.2 executable and its post-build source inputs."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import native
import protocol_capture as capture

sys.path.insert(0, str(native.ROOT / "scripts/benchmarks"))
import provenance


def build_evidence(receipt: Path, validation: Path) -> dict:
    build = capture.json_object(receipt.read_bytes())
    if (build.get("state"), build.get("exit_code"), build.get("command")) != (
            "finished", 0, ["cargo", "build", "--workspace", "--release", "--locked", "-j", "1"]):
        raise ValueError("release freeze requires a successful workspace release build")
    if Path(build["workdir"]).resolve() != native.ROOT or Path(build["cargo_target_dir"]).resolve() != native.ROOT / "target/build":
        raise ValueError("release build owner differs from this workspace")
    report = capture.json_object(validation.read_bytes())
    if report["totals"]["failed"] or not report["suite_count"] or not report["totals"]["passed"]:
        raise ValueError("release freeze requires a passing workspace validation report")
    if native.sha(Path(report["log"])) != report["log_sha256"]:
        raise ValueError("workspace validation log changed")
    log = receipt.with_suffix(".log")
    if "warning:" in log.read_text(encoding="utf-8"):
        raise ValueError("release build has unresolved warnings")
    return {"build_receipt": capture.identity(receipt), "build_log": capture.identity(log),
            "workspace_report": capture.identity(validation), "workspace_log_sha256": report["log_sha256"],
            "workspace_totals": report["totals"], "workspace_suite_count": report["suite_count"]}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--validation", type=Path, required=True)
    parser.add_argument("--persistence-probe", type=Path)
    args = parser.parse_args()
    output, executable = args.output.resolve(), args.executable.resolve()
    if output.exists() or not output.parent.is_dir() or not output.is_relative_to(native.ROOT / "target"):
        parser.error("output must be absent with an existing parent under target/")
    if executable != native.ROOT / "target/build/release/bcore.exe" or not executable.is_file():
        parser.error("select this workspace's built release bcore.exe")
    if args.persistence_probe is not None:
        args.persistence_probe = args.persistence_probe.resolve()
        if args.persistence_probe != native.ROOT / "target/build/release/examples/persistence_probe.exe" or not args.persistence_probe.is_file():
            parser.error("select this workspace's built persistence_probe.exe")
    version, _ = native.environment()
    evidence = build_evidence(args.build_receipt.resolve(), args.validation.resolve())
    expected_executable = capture.identity(executable)
    before = provenance.source_hashes(native.ROOT)
    output.mkdir()
    provenance.freeze_copy(Path(__file__), output / "freeze_release.py")
    source, sources = provenance.snapshot(native.ROOT, output, before)
    binary = output / "bin"
    binary.mkdir()
    frozen = binary / executable.name
    provenance.freeze_copy(executable, frozen, expected_executable["sha256"])
    provenance.verify_files(source, before)
    if provenance.source_hashes(native.ROOT) != before:
        raise ValueError("source inputs changed during release freeze")
    result = {"schema": "bcore.release-freeze.v1", "minecraft": version, "jar_sha256": native.JAR_SHA256,
              "authority": "post-build executable/source freeze with retained build and workspace validation",
              "evidence": evidence, "source_executable": expected_executable,
              "executable": capture.identity(frozen), "sources": sources,
              "freeze_script_sha256": native.sha(output / "freeze_release.py")}
    if args.persistence_probe is not None:
        probe = binary / args.persistence_probe.name
        provenance.freeze_copy(args.persistence_probe, probe)
        result["persistence_probe"] = capture.identity(probe)
    native.write_json(output / "release.json", result)
    print(json.dumps({"output": str(output), "executable": result["executable"],
                      "source_file_count": sources["source_file_count"],
                      "source_archive_sha256": sources["source_archive_sha256"],
                      "release_sha256": native.sha(output / "release.json")}, indent=2))


if __name__ == "__main__":
    main()
