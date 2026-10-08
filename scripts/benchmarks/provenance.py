"""Local, inspectable build inputs and tamper-checked benchmark artifacts."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import zipfile

JAVA_SOURCES = (
    "scripts/TreeReference.java", "scripts/NativeWorldgenRegistries.java",
    "scripts/jigsaw-reference/JigsawSupport.java",
    "scripts/density-materials-reference/DensityMaterialsReference.java",
    "scripts/benchmarks/NoiseFillBenchmark.java",
)
HARNESS_SOURCES = (*JAVA_SOURCES, "scripts/benchmarks/run.py",
                   "scripts/benchmarks/provenance.py",
                   "crates/bcore-worldgen/src/generation/benchmark.rs",
                   "crates/bcore-worldgen/examples/noise_bench.rs")


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path, value, *, exclusive=True):
    with Path(path).open("x" if exclusive else "w", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def source_hashes(root):
    """All workspace crate files, including assets/build scripts and new sources.

    Do not use git's index: an uncommitted optimization must be represented.
    Build/cache directories and private local environment files are not inputs.
    """
    root = Path(root).resolve()
    files = {root / p for p in JAVA_SOURCES}
    files.update(root / p for p in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml"))
    for directory in (root / "crates", root / "scripts/benchmarks"):
        for parent, dirs, names in os.walk(directory, followlinks=False):
            dirs[:] = sorted(d for d in dirs if d not in ("target", ".git", "__pycache__"))
            for directory_name in dirs:
                path = Path(parent) / directory_name
                if path.is_symlink() or not path.resolve().is_relative_to(root):
                    raise ValueError(f"source directory symlink is not snapshot-safe: {parent}/{directory_name}")
            for name in names:
                if name.startswith(".env") or name in ("credentials", "credentials.toml"):
                    continue
                path = Path(parent) / name
                if path.is_symlink() or not path.resolve().is_relative_to(root):
                    raise ValueError(f"source escapes workspace: {path}")
                files.add(path)
    # A copied config can both contain credentials and change meaning through
    # relative paths. This contract uses manifest-defined profiles instead.
    for name in (".cargo/config", ".cargo/config.toml"):
        if (root / name).exists():
            raise ValueError("project-local Cargo config is unsupported by this benchmark contract")
    return {p.relative_to(root).as_posix(): sha(p) for p in sorted(files)}


def verify_files(root, expected):
    for name, digest in expected.items():
        path = Path(root) / name
        if not path.is_file() or sha(path) != digest:
            raise ValueError(f"artifact changed or missing: {path}")


def freeze_copy(source, destination, expected=None):
    """Copy once, check both ends, then make accidental overwrites fail."""
    source, destination = Path(source), Path(destination)
    expected = expected or sha(source)
    with source.open("rb") as src, destination.open("xb") as dst:
        shutil.copyfileobj(src, dst)
    if sha(source) != expected or sha(destination) != expected:
        raise ValueError(f"artifact changed while copying: {source}")
    destination.chmod(stat.S_IRUSR | stat.S_IRGRP | stat.S_IROTH
                      | (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH if os.access(source, os.X_OK) else 0))
    return expected


def snapshot(root, output, expected):
    source = output / "source"
    source.mkdir()
    for name, digest in expected.items():
        destination = source / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        freeze_copy(root / name, destination, digest)
    if source_hashes(root) != expected:
        raise ValueError("working sources changed while snapshotting")
    manifest = output / "source-manifest.json"
    write_json(manifest, {"schema": 1, "files": expected})
    archive = output / "source.zip"
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as stream:
        for name in sorted(expected):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            stream.writestr(info, (source / name).read_bytes())
    verify_files(source, expected)
    return source, {"source_manifest_sha256": sha(manifest), "source_archive_sha256": sha(archive),
                    "source_file_count": len(expected), "source_sha256": expected,
                    "harness_source_sha256": {name: expected[name] for name in HARNESS_SOURCES}}


def controlled_environment():
    """Use pinned project toolchain/default release flags; never dump the environment.

    Cargo's cache/home and normal OS/linker discovery remain available. Remove
    ambient compiler/JVM/datapack overrides rather than recording arbitrary
    option strings (which can contain private data). Logs record actual commands.
    """
    env = dict(os.environ)
    names = {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS", "RUSTUP_TOOLCHAIN",
             "RUSTC", "RUSTDOC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
             "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "JDK_JAVAC_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH", "BCORE_DATAPACK"}
    names.update(k for k in env if k.startswith(("CARGO_PROFILE_", "CARGO_BUILD_", "CARGO_TARGET_"))
                 and k != "CARGO_TARGET_DIR")
    removed = sorted(name for name in names if name in env)
    for name in names:
        env.pop(name, None)
    env.update(CARGO_BUILD_JOBS="1", RAYON_NUM_THREADS="2", CARGO_INCREMENTAL="0")
    return env, {"policy": "project-pinned toolchain; default release flags; no ambient JVM/datapack overrides",
                 "removed_override_names": removed,
                 "fixed": {k: env[k] for k in ("CARGO_BUILD_JOBS", "RAYON_NUM_THREADS", "CARGO_INCREMENTAL")}}


def cargo_config_hashes(cwd, env):
    # Cargo can inherit configuration outside the archived project. Hash only:
    # copying/dumping user configuration could disclose registry credentials.
    candidates = set()
    for parent in (cwd, *cwd.parents):
        candidates.update(parent / ".cargo" / name for name in ("config", "config.toml"))
    home = Path(env.get("CARGO_HOME", Path.home() / ".cargo"))
    candidates.update(home / name for name in ("config", "config.toml"))
    return {str(p.resolve()): sha(p) for p in sorted(candidates) if p.is_file()}
