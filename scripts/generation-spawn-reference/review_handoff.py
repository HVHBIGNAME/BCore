"""Summarize actual saved-mob load/pairing behavior without changing captures."""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path

from verify_repeat import plain, verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path)
    parser.add_argument("--kind")
    parser.add_argument("--catalog", action="store_true")
    parser.add_argument("--examples", action="store_true", help="first saved/load/pairing example per kind")
    args = parser.parse_args()
    _, summary = verify(args.fixture)
    data = json.loads(args.fixture.read_text(encoding="utf-8"))
    catalog = data["handoff_catalog"]
    if args.examples:
        seen = set()
        for case in data["cases"]:
            for entity in case["entities"]:
                kind = entity["type"]
                if kind in seen or (args.kind and kind != "minecraft:" + args.kind):
                    continue
                seen.add(kind)
                print(json.dumps({"kind": kind, "saved": plain(entity["typed_nbt"]),
                                  "loaded": plain(entity["handoff"]["loaded"]["typed_nbt"]),
                                  "packets": entity["handoff"]["packets"]}, indent=2))
        return
    if args.catalog:
        for kind, fields in catalog["metadata"].items():
            if args.kind and kind != "minecraft:" + args.kind:
                continue
            print(kind)
            for field in fields:
                print(json.dumps(field, separators=(",", ":")))
            print("default_attributes", json.dumps(catalog["default_attributes"][kind], separators=(",", ":")))
        print("variant_registries", json.dumps({k: v for k, v in catalog["registries"].items() if k not in ("item", "attribute")}, separators=(",", ":")))
        return
    packets = Counter()
    metadata = defaultdict(lambda: defaultdict(set))
    attributes = defaultdict(set)
    differences = defaultdict(dict)
    for case in data["cases"]:
        for entity in case["entities"]:
            kind = entity["type"]
            if args.kind and kind != "minecraft:" + args.kind:
                continue
            h = entity["handoff"]
            before, after = plain(entity["typed_nbt"]), plain(h["loaded"]["typed_nbt"])
            for obj in [before, after]:
                obj["attributes"].sort(key=lambda a: a["id"])
            for key in before.keys() | after.keys():
                if before.get(key) != after.get(key):
                    differences[kind].setdefault(key, [before.get(key), after.get(key)])
            for packet in h["packets"]:
                packets[packet["class"]] += 1
                for entry in packet.get("entries", []):
                    metadata[kind][str(entry["index"])].add(entry["hex"])
                if "attributes" in packet:
                    attributes[kind].add(tuple(sorted(a["name"] for a in packet["attributes"])))
    print(json.dumps({"summary": summary, "packets": packets, "load_changes": differences,
                      "metadata_hex_values": {k: {i: sorted(v) for i, v in rows.items()} for k, rows in metadata.items()},
                      "attribute_sets": {k: sorted(v) for k, v in attributes.items()}}, indent=2))


if __name__ == "__main__":
    main()
