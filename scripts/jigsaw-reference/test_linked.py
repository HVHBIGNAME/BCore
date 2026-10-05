"""Compile the exported-library test against an already built worldgen library.

Use only while another owner's in-progress shared edits prevent Cargo building
the library. All linked artifacts are read-only. The normal verification command
remains cargo test --release -p bcore-worldgen --test jigsaw_reference -j 1,
using the caller's assigned CARGO_TARGET_DIR.
"""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--worldgen", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    libraries = list(args.artifacts.rglob("*.rlib"))
    directories = {p.parent for p in libraries}
    directories.update(p.parent for p in args.artifacts.rglob("*serde_derive*.dll"))
    args.output.mkdir(parents=True, exist_ok=True)
    executable = args.output / "jigsaw-linked.exe"
    command = ["rustc", "--edition=2021", "--test", str(ROOT / "crates/bcore-worldgen/tests/jigsaw_reference.rs"),
               "--crate-name", "jigsaw_reference", "-o", str(executable), "--extern", f"bcore_worldgen={args.worldgen.with_suffix('.rmeta')}"]
    command += ["--extern", f"bcore_worldgen={args.worldgen}"]
    for name in ["bcore_core", "serde", "serde_json", "md5", "sha2"]:
        matches = [p for p in libraries if p.name.startswith(f"lib{name}-")]
        if len(matches) != 1:
            raise ValueError(f"expected one {name} artifact, found {matches}")
        command += ["--extern", f"{name}={matches[0].with_suffix('.rmeta')}"]
        command += ["--extern", f"{name}={matches[0]}"]
    for directory in sorted(directories):
        command += ["-L", f"dependency={directory}"]
    print(f"Read-only library: {args.worldgen}", flush=True)
    subprocess.run(command, check=True)
    subprocess.run([str(executable), "--nocapture"], check=True)


if __name__ == "__main__":
    main()
