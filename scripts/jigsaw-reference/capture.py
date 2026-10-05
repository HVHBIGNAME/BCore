"""Extract/capture the pinned 26.1 jigsaw implementation in an isolated JVM.

No source implementation is copied. Templates are loaded through Minecraft's
ResourceManagerTemplateSource (including its data fixer), then serialized by the
native StructureTemplate. Pool/processor JSON and original asset hashes come
directly from the pinned JAR. Only this probe's target directory is written.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
KINDS = ("End", "Byte", "Short", "Int", "Long", "Float", "Double", "ByteArray",
         "String", "List", "Compound", "IntArray", "LongArray")


class NbtReader:
    def __init__(self, data: bytes):
        self.stream = io.BytesIO(data)

    def read(self, size: int) -> bytes:
        data = self.stream.read(size)
        if len(data) != size:
            raise ValueError("truncated NBT")
        return data

    def number(self, fmt: str):
        return struct.unpack(">" + fmt, self.read(struct.calcsize(fmt)))[0]

    def string(self) -> str:
        # Java modified UTF-8: NUL uses C0 80 and supplementary characters are
        # encoded as surrogate pairs. Preserve both in ordinary Unicode JSON.
        raw = self.read(self.number("H")).replace(b"\xc0\x80", b"\0")
        return raw.decode("utf-8", "surrogatepass").encode("utf-16", "surrogatepass").decode("utf-16")

    def payload(self, tag: int):
        if 1 <= tag <= 6:
            value = self.number(("b", "h", "i", "q", "f", "d")[tag - 1])
        elif tag == 8:
            value = self.string()
        elif tag in (7, 11, 12):
            size = self.number("i")
            if size < 0:
                raise ValueError("negative NBT array length")
            value = [self.number({7: "b", 11: "i", 12: "q"}[tag]) for _ in range(size)]
        elif tag == 9:
            element = self.number("B")
            size = self.number("i")
            if size < 0 or (element == 0 and size):
                raise ValueError("invalid NBT list")
            value = {"element_type": element, "values": [self.payload(element) for _ in range(size)]}
        elif tag == 10:
            value = {}
            while (kind := self.number("B")) != 0:
                name = self.string()
                value[name] = self.payload(kind)
        else:
            raise ValueError(f"unsupported NBT tag {tag}")
        return {KINDS[tag]: value}

    def root(self):
        tag = self.number("B")
        if tag != 10:
            raise ValueError("expected compound root")
        self.string()
        result = self.payload(tag)
        if self.stream.read(1):
            raise ValueError("trailing NBT bytes")
        return result


def plain(nbt):
    kind, value = next(iter(nbt.items()))
    if kind == "Compound":
        return {key: plain(child) for key, child in value.items()}
    if kind == "List":
        return [plain(child) for child in value["values"]]
    return value


def provenance(jar: Path, sources: list[Path]):
    digest = hashlib.sha256(jar.read_bytes()).hexdigest()
    if digest != JAR_SHA256:
        raise ValueError(f"wrong server JAR: {digest}")
    return {"minecraft": "26.1", "jar_sha256": digest,
            "probe_sha256": hashlib.sha256(b"".join(p.read_bytes() for p in sources)).hexdigest()}


def probe_sources(name: str):
    files = [ROOT / "scripts/TreeReference.java", ROOT / "scripts/NativeWorldgenRegistries.java",
            ROOT / "scripts/jigsaw-reference/JigsawSupport.java", ROOT / f"scripts/jigsaw-reference/{name}.java"]
    if name == "JigsawExpansionReference":
        files.insert(-1, ROOT / "scripts/jigsaw-reference/JigsawReference.java")
    return files


def decode_fixture(value):
    if isinstance(value, dict):
        return {key: NbtReader(base64.b64decode(child)).root() if key == "nbt" and isinstance(child, str)
                else decode_fixture(child) for key, child in value.items()}
    if isinstance(value, list):
        return [decode_fixture(child) for child in value]
    return value


def run_probe(args, name: str, probe_args=()):
    sources = probe_sources(name)
    header = provenance(args.jar, sources)
    args.build = args.build.resolve()
    args.jar = args.jar.resolve()
    args.build.mkdir(parents=True, exist_ok=True)
    subprocess.run([args.javac, "-d", str(args.build), *map(str, sources)], check=True)
    libraries = sorted((args.jar.parents[2] / "libraries").rglob("*.jar"))
    classpath = os.pathsep.join(map(str, [args.build, args.jar, *libraries]))
    result = subprocess.run([str(args.java.resolve()), "-Xmx3G", "-cp", classpath, name, str(args.jar), *probe_args],
                            cwd=args.build, capture_output=True, text=True, timeout=600)
    (args.build / f"{name}.stdout.log").write_text(result.stdout, encoding="utf-8")
    (args.build / f"{name}.stderr.log").write_text(result.stderr, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    if "Serialization errors:" in result.stdout + result.stderr:
        raise ValueError(f"native serialization failed; see {args.build / (name + '.stdout.log')}")
    marker = name.upper() + "="
    lines = [line.split(marker, 1)[1] for line in result.stdout.splitlines() if marker in line]
    if len(lines) != 1:
        raise ValueError(f"missing {marker}: {result.stdout}\n{result.stderr}")
    return header | json.loads(lines[0])


def extract(args):
    families = args.family or ["ancient_city", "village"]
    data = run_probe(args, "JigsawAssets", families)
    for entity in data["block_entities"].values():
        entity["nbt"] = NbtReader(base64.b64decode(entity["nbt"])).root()
    templates = {}
    for name, encoded in data.pop("template_nbt").items():
        root = NbtReader(base64.b64decode(encoded)).root()["Compound"]
        value = plain({"Compound": root})
        palettes = value.get("palettes", [value.get("palette", [])])
        blocks = []
        for typed, block in zip(root["blocks"]["List"]["values"], value["blocks"]):
            row = [*block["pos"], block["state"]]
            if "nbt" in typed["Compound"]:
                row.append(typed["Compound"]["nbt"])
            blocks.append(row)
        entities = []
        for typed, entity in zip(root["entities"]["List"]["values"], value["entities"]):
            entities.append({"pos": entity["pos"], "block_pos": entity["blockPos"],
                             "nbt": typed["Compound"]["nbt"]})
        templates[name] = {"size": value["size"], "palettes": palettes, "blocks": blocks,
                           "entities": entities, "data_version": value["DataVersion"]}
    data["templates"] = templates
    structures = {name.removeprefix("minecraft:") for name in data["structure_metadata"]}
    sets = {"ancient_city": "ancient_cities", "village": "villages", "pillager_outpost": "pillager_outposts"}
    structure_sets = {sets.get(family, family) for family in families}
    with zipfile.ZipFile(args.jar) as jar:
        assets = {}
        for path in sorted(jar.namelist()):
            if (path.startswith("data/minecraft/worldgen/template_pool/")
                    and (any("/" + family + "/" in path for family in families) or path.endswith("/empty.json"))
                    or path.startswith("data/minecraft/worldgen/processor_list/")
                    or path.startswith("data/minecraft/worldgen/structure/")
                    and Path(path).stem in structures
                    or path.startswith("data/minecraft/worldgen/structure_set/") and Path(path).stem in structure_sets
                    or path in ("data/minecraft/worldgen/placed_feature/sculk_patch_ancient_city.json", "data/minecraft/worldgen/configured_feature/sculk_patch_ancient_city.json")
                    or path.startswith("data/minecraft/tags/worldgen/biome/has_structure/")
                    and Path(path).stem in structures
                    or path.startswith("data/minecraft/tags/block/")) and path.endswith(".json"):
                raw = jar.read(path)
                assets[path] = {"sha256": hashlib.sha256(raw).hexdigest(), "value": json.loads(raw)}
        data["jar_assets"] = assets
        data["template_sources"] = {}
        for name in templates:
            path = f"data/minecraft/structure/{name.removeprefix('minecraft:')}.nbt"
            data["template_sources"][name] = {"path": path, "missing_from_jar": name in data["missing_templates"],
                "sha256": None if name in data["missing_templates"] else hashlib.sha256(jar.read(path)).hexdigest()}
    write_result(args, data)
    print(f"Extracted {len(templates)} templates, {len(data['states'])} states, {len(data['jar_assets'])} JSON assets")


def write_result(args, data):
    if args.verify:
        expected = json.loads(args.verify.read_text(encoding="utf-8"))
        if expected != data:
            raise ValueError(f"native recapture differs from {args.verify}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(data, separators=(",", ":"), ensure_ascii=False, allow_nan=False) + "\n")
    print(f"{'Verified' if args.verify else 'Wrote'} {args.output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["assets", "fixture", "expansion", "verify-recorded", "inspect", "bytecode", "summary"])
    parser.add_argument("--jar", type=Path, default=ROOT / "target/vanilla-775/versions/26.1/server-26.1.jar")
    parser.add_argument("--java", type=Path, default=ROOT / "target/jre25/jdk-25.0.4.1+1-jre/bin/java.exe")
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--build", type=Path, default=ROOT / "target/jigsaw-reference")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    parser.add_argument("--section", choices=["all", "cities"], default="all",
                        help="recapture only pending city cases, preserving other fixture sections")
    parser.add_argument("--base", type=Path, help="existing fixture for --section cities (defaults to --output)")
    parser.add_argument("--classes", nargs="*")
    parser.add_argument("--family", action="append", choices=["ancient_city", "village", "trail_ruins", "trial_chambers", "pillager_outpost"])
    args = parser.parse_args()
    provenance(args.jar, [])
    if args.mode == "assets":
        args.output = args.output or ROOT / "crates/bcore-worldgen/data/jigsaw_assets_26_1.json"
        extract(args)
    elif args.mode == "expansion":
        if args.output is None:
            parser.error("expansion requires a new --output")
        write_result(args, decode_fixture(run_probe(args, "JigsawExpansionReference")))
    elif args.mode == "fixture":
        args.output = args.output or ROOT / "crates/bcore-worldgen/data/jigsaw_reference_26_1.json"
        previous = json.loads((args.base or args.output).read_text(encoding="utf-8")) if args.section == "cities" else None
        data = run_probe(args, "JigsawReference", ["cities"] if previous is not None else [])
        data = decode_fixture(data)
        if previous is not None:
            if previous["jar_sha256"] != data["jar_sha256"]:
                raise ValueError("cannot merge captures from different server JARs")
            def city_key(case):
                return case["seed"], tuple(case["chunk"]), case["entire"], case.get("terrain", "stone")
            current = {city_key(case): case for case in data["cities"]}
            for old in previous["cities"]:
                key = city_key(old)
                new = current[key]
                # The registry/component repair may change saved inventories,
                # never already-verified block placement, ticks, marks or RNG.
                for field, value in old.items():
                    if field != "block_entities" and new[field] != value:
                        raise ValueError(f"city {key}: previously captured {field} changed")
                if len(old["block_entities"]) != len(new["block_entities"]):
                    raise ValueError(f"city {key}: block entity count changed")
            previous["cities"] = data["cities"]
            previous.setdefault("section_probe_sha256", {})["cities"] = data["probe_sha256"]
            data = previous
        write_result(args, data)
    elif args.mode == "verify-recorded":
        path = args.output or ROOT / "crates/bcore-worldgen/data/jigsaw_reference_26_1.json"
        expected = json.loads(path.read_text(encoding="utf-8"))
        log = (args.build / "JigsawReference.stdout.log").read_text(encoding="utf-8")
        if "Serialization errors:" in log:
            raise ValueError("recorded native capture has serialization errors")
        marker = "JIGSAWREFERENCE="
        lines = [line.split(marker, 1)[1] for line in log.splitlines() if marker in line]
        if len(lines) != 1:
            raise ValueError("expected exactly one completed native capture")
        captured = decode_fixture(json.loads(lines[0]))
        header = provenance(args.jar, probe_sources("JigsawReference"))
        if expected["jar_sha256"] != header["jar_sha256"]:
            raise ValueError("recorded capture JAR differs")
        sections = ["cities"] if args.section == "cities" else ["assembly", "placement", "transforms", "cities"]
        for section in sections:
            source_hash = expected.get("section_probe_sha256", {}).get(section, expected["probe_sha256"])
            if source_hash != header["probe_sha256"]:
                raise ValueError(f"{section}: probe sources differ from the captured version")
            if expected[section] != captured[section]:
                raise ValueError(f"{section}: fixture differs from recorded native output")
            print(f"Verified {len(captured[section])} {section} against recorded native output and probe/JAR hashes")
    elif args.mode == "bytecode":
        args.build.mkdir(parents=True, exist_ok=True)
        for name in args.classes:
            result = subprocess.run(["javap", "-classpath", str(args.jar), "-c", "-p", name], check=True, capture_output=True, text=True)
            path = args.build / (name.rsplit(".", 1)[-1] + ".txt")
            path.write_text(result.stdout, encoding="utf-8")
            print(path)
    elif args.mode == "summary":
        path = args.output or ROOT / "crates/bcore-worldgen/data/jigsaw_assets_26_1.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        print("jar_sha256", data["jar_sha256"], "probe_sha256", data["probe_sha256"])
        if "section_probe_sha256" in data:
            print("section_probe_sha256", json.dumps(data["section_probe_sha256"], sort_keys=True))
        if "assembly" in data:
            print("COUNTS", "assemblies", len(data["assembly"]), "pieces", sum(len(c["pieces"]) for c in data["assembly"]),
                  "placements", len(data["placement"]), "transforms", len(data["transforms"]), "cities", len(data.get("cities", [])))
            for case in data.get("cities", []):
                print("city", case["seed"], case.get("terrain", "stone"), "entire", case["entire"], "writes", case["write_count"], "positions", case["state_count"],
                      "block_entities", len(case["block_entities"]), "ticks", len(case["ticks"]),
                      "marks", sum(row[3] for row in case["marks"]), "rng", case["next_i64"])
            for section in ["placement", "cities"]:
                cases = data.get(section, [])
                print(section, "totals", "writes", sum(c["write_count"] for c in cases),
                      "positions", sum(c["state_count"] for c in cases), "block_entities", sum(len(c["block_entities"]) for c in cases),
                      "ticks", sum(len(c["ticks"]) for c in cases), "marks", sum(row[3] for c in cases for row in c.get("marks", [])))
            for case in data.get("cities", []):
                inventories = [dict(pos=e["pos"], nbt=plain(e["nbt"])) for e in case["block_entities"]
                               if e["nbt"].get("Compound", {}).get("Items", {}).get("List", {}).get("values")]
                print("city inventories", case["seed"], case.get("terrain", "stone"), case["chunk"], json.dumps(inventories, sort_keys=True))
            for case in data["assembly"]:
                print(case["structure"], case["seed"], case.get("generation_point"), len(case["pieces"]), case["next_i64"])
            print("first piece", json.dumps(plain(data["assembly"][0]["pieces"][0]["nbt"]), indent=2))
            for case in data["placement"][:2]:
                print("placement", case["template"], case["write_count"], case["state_count"])
                print("block_entities", [plain(e["nbt"]) for e in case["block_entities"]])
            return
        def visit(value, field):
            if isinstance(value, dict):
                if field in value:
                    yield value[field]
                for child in value.values():
                    yield from visit(child, field)
            elif isinstance(value, list):
                for child in value:
                    yield from visit(child, field)
        pools = {k: v["value"] for k, v in data["jar_assets"].items() if "/template_pool/" in k}
        processors = set(p for p in visit(pools, "processors") if isinstance(p, str))
        print("bytes", path.stat().st_size, "templates", len(data["templates"]), "pools", len(pools))
        print("states", len(data["states"]), "block entity defaults", len(data["block_entities"]), "JSON assets", len(data["jar_assets"]))
        print("missing templates", data["missing_templates"])
        for name, metadata in sorted(data["structure_metadata"].items()):
            config = data["jar_assets"]["data/minecraft/worldgen/structure/" + name.removeprefix("minecraft:") + ".json"]["value"]
            print("structure", name, "metadata", metadata, "terrain adaptation", config["terrain_adaptation"])
        print("features", sorted(set(visit(pools, "feature"))))
        print("processor lists", sorted(processors))
        for name in sorted(processors):
            value = data["jar_assets"]["data/minecraft/worldgen/processor_list/" + name.removeprefix("minecraft:") + ".json"]["value"]
            print(name, sorted(set(visit(value, "processor_type"))), sorted(set(visit(value, "predicate_type"))))
        print("entity kinds", sorted({e["nbt"]["Compound"]["id"]["String"] for t in data["templates"].values() for e in t["entities"]}))
    else:
        with zipfile.ZipFile(args.jar) as jar:
            for path in sorted(jar.namelist()):
                if path.startswith("data/minecraft/worldgen/") and path.endswith(".json") and (
                    path.endswith("/ancient_city.json") or path.endswith("/village_plains.json")
                    or "/ancient_city/" in path or "processor_list/ancient" in path):
                    print(path, jar.read(path).decode())


if __name__ == "__main__":
    main()
