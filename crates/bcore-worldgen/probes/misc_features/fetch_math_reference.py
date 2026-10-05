"""Fetch public OpenJDK numeric-stub references into isolated analysis artifacts."""

import hashlib
import json
from pathlib import Path
import urllib.request


ROOT = Path(__file__).resolve().parents[4]
OUT = ROOT / "target/full-parity-20261002/misc-math-source"
BASE = "https://raw.githubusercontent.com/openjdk/jdk/jdk-25%2B36/src/hotspot/cpu/x86/"


def main():
    OUT.mkdir(exist_ok=True)
    sources = []
    for name in ("stubGenerator_x86_64_sin.cpp", "stubGenerator_x86_64_cos.cpp", "stubGenerator_x86_64_constants.cpp"):
        url = BASE + name
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
        path = OUT / name
        if path.exists() and path.read_bytes() != data:
            raise ValueError(f"Refusing to replace different analysis source: {path}")
        path.write_bytes(data)
        sources.append({"url": url, "sha256": hashlib.sha256(data).hexdigest(), "path": str(path.relative_to(ROOT))})
    print(json.dumps(sources, indent=2))


if __name__ == "__main__":
    main()
