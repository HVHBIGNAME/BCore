"""Read the pinned native class/method bytecode without decompiler substitutes."""

import argparse
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
JAR = ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("owner")
    parser.add_argument("methods", nargs="*", help="literal method-name fragments")
    args = parser.parse_args()
    owner = args.owner if args.owner.startswith("net.") else "net.minecraft." + args.owner
    text = subprocess.run(
        ["javap", "-classpath", str(JAR), "-c", "-p", owner],
        capture_output=True, text=True, check=True,
    ).stdout
    if not args.methods:
        print(text)
        return
    sections = re.split(r"(?=^  \S.*(?:;|\{)$)", text, flags=re.M)
    for section in sections:
        header = section.splitlines()[0] if section else ""
        if any(method in header for method in args.methods):
            print(section.rstrip())


if __name__ == "__main__":
    main()
