"""Package a completed capture byte-for-byte; neither normalization nor recapture."""

import argparse
import json
from pathlib import Path
import zipfile

from oracle import sha, summarize


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("native fixtures are immutable: choose a new destination")
    audit = summarize(args.capture, verify=True)
    files = [args.capture / p for p in ("config.json", "capture.json", "provenance.json", "events.jsonl", "block-states.json", "server.log")]
    files.extend(sorted((args.capture / "blobs").glob("*.gz")))
    files.extend(sorted((args.capture / "build").glob("*.java")))
    files.extend(sorted((args.capture / "build").glob("*.py")))
    files.extend(args.capture / "server" / p for p in ("server.properties", "eula.txt"))
    manifest = {"schema": 1, "native_capture": str(args.capture.resolve()), "audit": audit,
                "files_sha256": {p.relative_to(args.capture).as_posix(): sha(p) for p in files}}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.output, "x", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for file in files:
            archive.write(file, file.relative_to(args.capture).as_posix())
        archive.writestr("fixture.json", json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"fixture": str(args.output), "bytes": args.output.stat().st_size,
                      "sha256": sha(args.output), "files": len(files)}, indent=2))


if __name__ == "__main__":
    main()
