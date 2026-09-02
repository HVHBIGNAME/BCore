"""Remove generated chunk saves so tests and servers regenerate terrain.

`cargo test` and a dev server write `world/chunks/*.bcc` relative to their working
directory, which means saves accumulate in both the repo root and
`crates/bcore-protocol/` (tests run with the crate as cwd). Those files are
reproducible from the seed, so wiping them is always safe — and *necessary* after
a generator change, because a stale save is loaded in preference to regenerating
and would silently serve the old terrain.

usage: python scripts/clean_world.py [extra_dir ...]
"""

from __future__ import annotations

import pathlib
import shutil
import stat
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Every place a `world/` directory can legitimately appear.
CANDIDATES = [
    ROOT / "world",
    ROOT / "crates" / "bcore-protocol" / "world",
    ROOT / "crates" / "bcore" / "world",
    ROOT / "crates" / "bcore-worldgen" / "world",
]


def force_writable(func, path, _exc_info):
    """rmtree onerror hook: clear the read-only bit and retry (Windows)."""
    try:
        pathlib.Path(path).chmod(stat.S_IWRITE)
        func(path)
    except OSError as exc:
        print(f"  ! could not remove {path}: {exc}")


def main() -> None:
    targets = list(CANDIDATES)
    for extra in sys.argv[1:]:
        targets.append(pathlib.Path(extra))

    removed = 0
    for target in targets:
        if not target.exists():
            continue
        chunk_files = list(target.rglob("*.bcc")) + list(target.rglob("*.bcc.tmp"))
        print(f"removing {target}  ({len(chunk_files)} chunk files)")
        shutil.rmtree(target, onerror=force_writable)
        removed += 1

    if removed == 0:
        print("no generated worlds found; nothing to clean")
    else:
        print(f"cleaned {removed} world director{'y' if removed == 1 else 'ies'}")

    # Report anything left behind, so a stale save can never hide.
    leftovers = [p for p in ROOT.rglob("*.bcc") if "target" not in p.parts]
    if leftovers:
        print(f"WARNING: {len(leftovers)} chunk files remain:")
        for path in leftovers[:10]:
            print("   ", path.relative_to(ROOT))


if __name__ == "__main__":
    main()
