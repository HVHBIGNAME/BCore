"""Summarize complete libtest JSON logs without truncating their retained evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def report(path: Path) -> dict:
    raw = path.read_bytes()
    target = "unknown"
    suites, failures = [], []
    for line in raw.decode("utf-8", "strict").splitlines():
        text = line.strip()
        if text.startswith(("Running ", "Doc-tests ")):
            target = text.split(" (", 1)[0]
        if not text.startswith("{"):
            continue
        event = json.loads(text)
        if event.get("type") == "suite" and event.get("event") in ("ok", "failed"):
            suites.append({"target": target, **event})
        if event.get("type") == "test" and event.get("event") == "failed":
            failures.append({"target": target, **event})
    return {"log": str(path.resolve()), "log_sha256": hashlib.sha256(raw).hexdigest(),
            "totals": {key: sum(suite[key] for suite in suites)
                       for key in ("passed", "failed", "ignored", "filtered_out")},
            "suite_count": len(suites), "suites": suites, "failures": failures}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--target", help="literal target-name filter for the summary")
    args = parser.parse_args()
    result = report(args.log)
    if args.output is not None:
        if args.output.exists() or not args.output.parent.is_dir():
            parser.error("output must be absent with an existing parent")
        with args.output.open("x", encoding="utf-8") as stream:
            json.dump(result, stream, indent=2, ensure_ascii=False, allow_nan=False)
            stream.write("\n")
    print(json.dumps({key: result[key] for key in ("log", "log_sha256", "totals", "suite_count")}, indent=2))
    for failure in result["failures"]:
        if args.target is not None and args.target not in failure["target"]:
            continue
        lines = [line for line in failure.get("stdout", "").splitlines() if line][:8]
        print(f"{failure['target']} :: {failure['name']}")
        for line in lines:
            print("  " + line[:600])


if __name__ == "__main__":
    main()
