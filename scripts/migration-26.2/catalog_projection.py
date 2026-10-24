"""Inspect an exact JSON Pointer in a captured catalog without modifying it."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re


def resolve_pointer(value, pointer: str):
    if pointer and not pointer.startswith("/"):
        raise ValueError("JSON Pointer must be empty or start with /")
    for token in pointer.split("/")[1:]:
        if re.search(r"~(?:[^01]|$)", token):
            raise ValueError(f"invalid JSON Pointer escape: {token}")
        key = token.replace("~1", "/").replace("~0", "~")
        if isinstance(value, list):
            if re.fullmatch(r"0|[1-9][0-9]*", key) is None or int(key) >= len(value):
                raise ValueError(f"JSON Pointer array index is unavailable: {key}")
            value = value[int(key)]
        elif isinstance(value, dict) and key in value:
            value = value[key]
        else:
            raise ValueError(f"JSON Pointer member is unavailable: {key}")
    return value


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("catalog", type=Path)
    parser.add_argument("pointer", nargs="?", default="")
    parser.add_argument("--keys", action="store_true")
    args = parser.parse_args()
    raw = args.catalog.read_bytes()
    try:
        value = resolve_pointer(json.loads(raw), args.pointer)
    except ValueError as error:
        parser.error(str(error))
    if args.keys:
        if isinstance(value, dict):
            value = list(value)
        elif isinstance(value, list):
            value = {"length": len(value)}
        else:
            parser.error("--keys requires an object or array")
    print(json.dumps({"catalog_sha256": hashlib.sha256(raw).hexdigest(),
                      "pointer": args.pointer, "keys_only": args.keys,
                      "value": value}, indent=2, ensure_ascii=False, allow_nan=False))


if __name__ == "__main__":
    main()
