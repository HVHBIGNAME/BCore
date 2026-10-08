"""Package only original-server fossil boundaries; no Rust output is an input."""
import argparse
import hashlib
import itertools
import json
from pathlib import Path
import struct
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/native-generation-reference"))
from oracle import decode_snapshot, load_events, sha, state_key, validate_stages
from feature_case import heightmaps


def rle(values):
    return [[sum(1 for _ in group), value] for value, group in itertools.groupby(values)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--bundle", action="store_true", help="also create the two new production/test data files")
    args = parser.parse_args()
    out = args.output.resolve()
    if not out.is_relative_to(ROOT / "target") or out.exists():
        parser.error("choose a new directory under target")
    capture = args.capture.resolve()
    events = load_events(capture)
    _, pending = validate_stages(events)
    if pending:
        raise ValueError("fossil regression requires a completed native history")
    native = json.loads((capture / "capture.json").read_text())
    assert native["events_sha256"] == sha(capture / "events.jsonl")
    assert native["provenance_sha256"] == sha(capture / "provenance.json")
    provenance = json.loads((capture / "provenance.json").read_text())
    assets = json.loads(args.assets.read_text())
    assert assets["jar_sha256"] == provenance["jar_sha256"]
    registry = {state_key(s): s["id"] for s in json.loads((capture / "block-states.json").read_text())}
    by_id = {event["seq"]: event for event in events}
    boundaries = {}
    for event in events:
        if event["event"] != "feature_snapshot" or event["feature"] not in ("minecraft:fossil_upper", "minecraft:fossil_lower"):
            continue
        key = (*event["source"], event["feature"])
        if event["boundary"] in boundaries.setdefault(key, {}):
            raise ValueError("duplicate feature boundary")
        boundaries[key][event["boundary"]] = event
    snapshots = {}

    def snapshot(raw):
        key = raw["nbt"].split(".")[0] + ":" + raw["extra"].split(".")[0]
        if key not in snapshots:
            data = decode_snapshot(capture, raw, registry)
            assert data["min_y"] == -64 and len(data["states"]) == 98304 and len(data["biomes"]) == 1536
            maps = heightmaps(data["extra"]["heightmaps_live"], data["min_y"])
            marks = [[packed & 15, (section - 4) * 16 + ((packed >> 4) & 15), (packed >> 8) & 15]
                     for section, values in enumerate(data["nbt"].get("PostProcessing", [])) for packed in values]
            snapshots[key] = {"pos": raw["pos"], "states": rle(data["states"]), "biomes": rle(data["biomes"]),
                "states_sha256": hashlib.sha256(struct.pack("<98304I", *data["states"])).hexdigest(),
                "world_surface_wg": maps["WORLD_SURFACE_WG"], "ocean_floor_wg": maps["OCEAN_FLOOR_WG"],
                "postprocessing": marks, "native_snapshot": raw}
        return key

    cases = []
    for key, pair in sorted(boundaries.items()):
        assert set(pair) == {"before", "after"}
        entry = by_id[pair["before"]["feature_event"]]
        exit = by_id[pair["after"]["feature_event"]]
        span = [e for e in events[entry["seq"]:exit["seq"] - 1] if e.get("stage_id") == entry["stage_id"]]
        effects = [e for e in span if e["event"] == "world_entity"]
        if effects:
            raise ValueError("native fossil unexpectedly generated an entity")
        cases.append({"source": list(key[:2]), "feature": key[2], "entry": entry, "exit": exit,
            "before": [snapshot(s) for s in pair["before"]["chunks"]],
            "after": [snapshot(s) for s in pair["after"]["chunks"]],
            "writes": [[*e["pos"], e["state"], e["flags"]] for e in span if e["event"] == "world_set_block" and e["accepted"]],
            "height_queries": [[e["heightmap"], *e["xz"], e["returned"]] for e in span if e["event"] == "world_height_query"],
            "raw_ticks": [e for e in span if e["event"] == "world_tick_request"]})
    data = {"schema": 1, "minecraft": "26.1", "jar_sha256": provenance["jar_sha256"],
        "seed": json.loads((capture / "config.json").read_text())["seed"],
        "native_provenance_sha256": native["provenance_sha256"], "native_events_sha256": native["events_sha256"],
        "native_sources": provenance["sources"], "exporter_sha256": sha(Path(__file__)),
        "decoder_sha256": sha(ROOT / "scripts/native-generation-reference/oracle.py"),
        "scope": "Actual native placed-feature before/after 3x3 block/biome/WG/ordered-mark snapshots, writes and copied RNG. RLE is lossless Y/Z/X; raw native NBT and live-map blob digests are retained.",
        "cases": cases, "snapshots": snapshots}
    encoded = (json.dumps(data, separators=(",", ":"), sort_keys=True) + "\n").encode()
    out.mkdir()
    (out / "fossil_history_26_1.json").write_bytes(encoded)
    (out / "pack_history.py").write_bytes(Path(__file__).read_bytes())
    if args.bundle:
        bundle = ROOT / "crates/bcore-worldgen/data"
        for name, contents in [("fossil_assets_26_1.json", args.assets.read_bytes()), ("fossil_history_26_1.json", encoded)]:
            with (bundle / name).open("xb") as stream:
                stream.write(contents)
    summary = {"cases": len(cases), "successful": sum(c["exit"]["result"] for c in cases),
        "snapshots": len(snapshots), "writes": sum(len(c["writes"]) for c in cases),
        "raw_ticks": sum(len(c["raw_ticks"]) for c in cases),
        "fixture_sha256": hashlib.sha256(encoded).hexdigest(), "assets_sha256": sha(args.assets)}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
