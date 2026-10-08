"""Verify two independently bootstrapped captures and package immutable evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify(path):
    manifest = json.loads((path / "manifest.json").read_text(encoding="utf-8"))
    for name, digest in manifest.items():
        if sha(path / name) != digest:
            raise ValueError(f"modified native evidence: {path / name}")
    data = json.loads((path / "observations.json").read_text(encoding="utf-8"))
    if data["schema"] != 1 or len(data["cases"]) != 39 or data["gameplay_ticks"] != 0:
        raise ValueError("incomplete lifecycle capture")
    names = [case["name"] for case in data["cases"]]
    if len(set(names)) != 39:
        raise ValueError("duplicate lifecycle case name")
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("first", type=Path)
    parser.add_argument("repeat", type=Path)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    first, repeat = args.first.resolve(), args.repeat.resolve()
    if first == repeat:
        parser.error("independent captures must use different directories/processes")
    data = verify(first)
    if data != verify(repeat):
        raise ValueError("independent native observations differ")
    a = json.loads((first / "provenance.json").read_text(encoding="utf-8"))
    b = json.loads((repeat / "provenance.json").read_text(encoding="utf-8"))
    for key in ["jar_sha256", "source_sha256", "runner_sha256", "server_properties"]:
        if a[key] != b[key]:
            raise ValueError(f"native capture provenance differs: {key}")
    if args.fixture.exists() or args.archive.exists():
        raise FileExistsError("never overwrite a published fixture/archive")
    data["evidence"] = {"captures": [str(first.relative_to(ROOT)), str(repeat.relative_to(ROOT))],
                        "manifests_sha256": [sha(first / "manifest.json"), sha(repeat / "manifest.json")],
                        "observations_sha256": sha(first / "observations.json"),
                        "source_sha256": a["source_sha256"]}
    args.archive.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.archive, "x", zipfile.ZIP_DEFLATED) as archive:
        for label, root in [("first", first), ("repeat", repeat)]:
            for path in sorted(root.iterdir()):
                if path.is_file():
                    archive.write(path, f"{label}/{path.name}")
            for path in sorted((root / "build").glob("*.java")):
                archive.write(path, f"{label}/probe/{path.name}")
            archive.write(root / "build/capture.py", f"{label}/probe/capture.py")
    data["evidence"]["archive_sha256"] = sha(args.archive)
    args.fixture.parent.mkdir(parents=True, exist_ok=True)
    with args.fixture.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(data, separators=(",", ":"), ensure_ascii=False, allow_nan=False) + "\n")
    print(json.dumps({"fixture": str(args.fixture), "cases": 39, "fixture_sha256": sha(args.fixture),
                      "archive": str(args.archive), "archive_sha256": sha(args.archive)}, indent=2))


if __name__ == "__main__":
    main()
