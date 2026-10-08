"""Render the README/Pages SVG dashboard from published, versioned measurements.

No plotting dependencies or remote image service. --check fails on stale graphics.
--benchmark imports an already-completed, hash-matched benchmark into a new JSON
artifact before rendering. It never changes an existing measurement.
"""
import argparse
import hashlib
import html
import json
import math
from pathlib import Path
import re
import statistics
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
METRICS = ROOT / "docs/metrics"
ASSETS = ROOT / "site/assets"
CHECKPOINT = METRICS / "checkpoint-2026-10-08.json"
BENCHMARK = METRICS / "noise-fill-2026-10-05.json"
BG, CARD, LINE = "#0b1220", "#111e31", "#26374e"
FG, MUTED = "#e8f0fc", "#a7bad2"
TEAL, BLUE, VIOLET, AMBER = "#50e3c2", "#73aaff", "#b89bff", "#f5c36c"


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def text(x, y, value, size=16, color=FG, weight=400, anchor="start", extra=""):
    return (f'<text x="{x}" y="{y}" font-size="{size}" fill="{color}" '
            f'font-weight="{weight}" text-anchor="{anchor}" {extra}>{html.escape(str(value))}</text>')


def rect(x, y, w, h, fill=CARD, radius=12, stroke=LINE, extra=""):
    return (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{radius}" '
            f'fill="{fill}" stroke="{stroke}" {extra}/>')


def line(x1, y1, x2, y2, color=LINE, width=1):
    return f'<path d="M{x1},{y1} L{x2},{y2}" fill="none" stroke="{color}" stroke-width="{width}"/>'


def document(height, title, description, elements):
    return "\n".join([
        '<svg xmlns="http://www.w3.org/2000/svg" width="1000" '
        f'height="{height}" viewBox="0 0 1000 {height}" role="img" aria-labelledby="title desc">',
        f'<title id="title">{html.escape(title)}</title>',
        f'<desc id="desc">{html.escape(description)}</desc>',
        '<defs><linearGradient id="background" x1="0" y1="0" x2="1" y2="1">'
        '<stop stop-color="#13243a"/><stop offset="1" stop-color="#0b1220"/></linearGradient>'
        '<pattern id="grid" width="32" height="32" patternUnits="userSpaceOnUse">'
        '<path d="M32,0 H0 V32" fill="none" stroke="#547294" stroke-opacity=".12"/>'
        '</pattern></defs>',
        f'<g font-family="Segoe UI, Ubuntu, Arial, sans-serif">',
        rect(1, 1, 998, height - 2, "url(#background)", 20),
        *elements, '</g></svg>', "",
    ])


def cube(x, y, size, height, color):
    s, h = size, height
    return [
        f'<path d="M{x + s},{y + s / 2} l{-s},{s / 2} v{h} l{s},{-s / 2} Z" fill="#1b354a" stroke="{color}" stroke-opacity=".5"/>',
        f'<path d="M{x - s},{y + s / 2} l{s},{s / 2} v{h} l{-s},{-s / 2} Z" fill="#183047" stroke="{color}" stroke-opacity=".4"/>',
        f'<path d="M{x},{y} l{s},{s / 2} l{-s},{s / 2} l{-s},{-s / 2} Z" fill="{color}" fill-opacity=".18" stroke="{color}"/>',
    ]


def overview(checkpoint):
    t = checkpoint["totals"]
    light_matches = t["light_scored_requests"] - t["light_mismatching_requests"]
    e = [rect(660, 18, 320, 158, "url(#grid)", 12, "none"),
         text(38, 45, "INDEPENDENT IMPLEMENTATION / RUST", 12, TEAL, 600, extra='letter-spacing="2"'),
         text(36, 112, "BCore", 66, FG, 700),
         text(39, 146, "Building native worlds. Checking every boundary.", 20, MUTED),
         rect(772, 28, 190, 27, "#233248", 13, "none"),
         text(867, 47, "JAVA 26.1  ·  PROTOCOL 775", 11, BLUE, 600, "middle")]
    e += cube(860, 74, 38, 58, TEAL) + cube(922, 113, 23, 29, BLUE) + cube(810, 124, 22, 20, VIOLET)
    cards = [(str(checkpoint["tests"]["passed"]), "tests passed", f'{checkpoint["tests"]["failed"]} failed · {checkpoint["tests"]["ignored"]} ignored', TEAL),
             (str(t["requests"]), "native requests", "matched execution histories", BLUE),
             (str(t["histories"]), "history scenarios", "adjacency · repeats · stages", VIOLET),
              (str(light_matches), "light snapshots match", "scored storage and bytes", TEAL)]
    for i, (value, label, detail, color) in enumerate(cards):
        x = 36 + i * 236
        e += [rect(x, 178, 222, 112), rect(x + 16, 194, 4, 28, color, 2, "none"),
              text(x + 32, 224, value, 36, color, 700), text(x + 17, 251, label, 16, FG, 600),
              text(x + 17, 274, detail, 11, MUTED)]
    e += [text(39, 322, "ALPHA / Terrain parity first. FULL conversion and remaining worldgen families are next.", 13, AMBER),
          text(960, 346, f'Verified checkpoint · {checkpoint["published_date"]}', 11, MUTED, anchor="end")]
    return document(364, "BCore verified development checkpoint",
                    f'{checkpoint["tests"]["passed"]} passing tests, {t["requests"]} native requests in {t["histories"]} histories. '
                     f'{light_matches} matching scored light snapshots. Full parity remains incomplete.', e)


def accuracy(checkpoint, historical):
    t = checkpoint["totals"]
    n = t["requests"]
    e = [text(36, 46, "Generation accuracy", 28, FG, 650),
         text(36, 73, "Measured outputs, with the sample boundaries kept visible.", 14, MUTED),
         rect(30, 96, 455, 351), rect(503, 96, 467, 351),
         text(50, 125, "LATEST / MATCHED NATIVE HISTORIES", 12, TEAL, 600, extra='letter-spacing="1"'),
          text(50, 160, str(t["state_differences"]), 40, TEAL, 700), text(87, 157, "block-state differences", 19, FG, 600),
         text(50, 184, f'{t["state_observations"]:,} observations · repeated snapshots included', 12, MUTED)]
    rows = [r for history in checkpoint["histories"] for r in history["requests"]]
    categories = [("Block snapshots", sum(r["states"]["mismatches"] == 0 for r in rows), n),
                  ("Quart-biome snapshots", sum(r["biomes"]["mismatches"] == 0 for r in rows), n),
                  ("Source execution order", sum(r["source_order_differences"] == 0 for r in rows), n),
                  ("Scored light snapshots", t["light_scored_requests"] - t["light_mismatching_requests"], t["light_scored_requests"])]
    for i, (label, matches, total) in enumerate(categories):
        y = 216 + i * 52
        e += [text(50, y, label, 14), text(463, y, f"{matches}/{total}", 14, TEAL, 600, "end"),
              rect(50, y + 9, 411, 7, LINE, 3, "none"),
              rect(50, y + 9, 411 * matches / total, 7, TEAL, 3, "none")]
    meta = t["metadata_field_differences"]
    e += [text(50, 424, f'Structures: {meta["structures"]} differences · ordered marks: {meta["postprocessing"]}', 12, MUTED),
          text(525, 125, "HISTORICAL / 2026-10-02 REGION SAMPLE", 12, BLUE, 600, extra='letter-spacing=".7"')]
    for i, row in enumerate(historical["samples"]):
        y = 163 + i * 72
        pct = row["equal_states"] / row["cells"]
        label = f'Centre ({row["center"][0]}, {row["center"][1]})'
        e += [text(525, y, label, 14), text(944, y, f"{pct:.2%}", 15, BLUE, 600, "end"),
              rect(525, y + 12, 417, 10, LINE, 5, "none"),
              rect(525, y + 12, round(417 * pct, 2), 10, BLUE, 5, "none")]
    matches = sum(row["equal_states"] for row in historical["samples"])
    total = sum(row["cells"] for row in historical["samples"])
    e += [text(525, 389, f"{matches / total:.2%}", 30, BLUE, 700),
          text(653, 386, f"{matches:,} / {total:,} states", 14),
          text(525, 421, "Includes air. Older executable + unmatched capture history.", 12, MUTED),
          rect(30, 466, 940, 93, "#272536", 12, "#5c4d39"),
          text(50, 493, "What is still different?", 16, AMBER, 650),
           text(50, 518, f'{meta["block_entity_payloads"]} NBT field differences · {t["wg_presence_differences"]} WG-presence differences across repeated snapshots.', 13, MUTED),
           text(50, 541, "Ticket-driven FULL conversion, ticking, other features and dimensions remain incomplete.", 13, MUTED),
          text(36, 589, "These are different evaluation scopes — neither is a percentage of the whole generator implemented.", 12, MUTED)]
    return document(614, "BCore generation accuracy and remaining differences",
                     f'{categories[0][1]}/{n} matching block snapshots and {categories[1][1]}/{n} biome snapshots. '
                     f'{categories[3][1]}/{categories[3][2]} scored light snapshots match. '
                    f'Historical three-region block match {matches / total:.2%}. '
                    f'{t["metadata_field_differences"]["block_entity_payloads"]} NBT field and {t["wg_presence_differences"]} WG-presence differences remain.', e)


def fixes(checkpoint):
    e = [text(36, 43, "From witness to fix", 27, FG, 650),
         text(36, 70, "Same request histories before and after the integration fixes.", 14, MUTED)]
    labels = {"fossil-history-native-01": ("Fossils / feature boundaries", "Shared RNG, templates and cross-chunk shapes"),
              "fossil-full-native-01": ("Fossils / FULL requests", "Repeated and adjacent native requests"),
              "desert-pyramid-history-01": ("Desert pyramids", "Native geometry, cellar and archaeology")}
    description = []
    for i, row in enumerate(checkpoint["before_after"]):
        x, y = 30, 94 + i * 119
        title, detail = labels.get(row["history"], (row["history"], "Matched native request history"))
        before, after = row["before_state_differences"], row["after_state_differences"]
        e += [rect(x, y, 940, 103), text(x + 20, y + 30, title, 18, FG, 600),
              text(x + 20, y + 56, detail, 12, MUTED),
              text(x + 20, y + 82, f'{row["requests"]} requests · repeated snapshots included', 12, MUTED),
              text(680, y + 56, f"{before:,}", 38, AMBER, 650, "end"),
              text(722, y + 51, "→", 27, MUTED, anchor="middle"),
              text(768, y + 56, f"{after:,}", 38, TEAL, 650),
              text(786, y + 83, "state differences", 12, MUTED, anchor="middle")]
        description.append(f"{title}: {before} to {after} across {row['requests']} requests")
    footer = 108 + len(checkpoint["before_after"]) * 119
    e += [text(36, footer, "Counts sum request snapshots; they are not counts of unique world positions or overall completion.", 12, MUTED)]
    return document(footer + 24, "Native-history regression fixes", "; ".join(description), e)


def nice_max(value):
    magnitude = 10 ** math.floor(math.log10(value))
    return math.ceil(value / magnitude) * magnitude


def performance(data):
    groups = sorted({row["workers"] for row in data["summary"]})
    max_value = nice_max(max(len(data["chunks"]) / row["min_process_median_seconds"] for row in data["summary"]) * 1.05)
    chart_x, chart_w = 224, 650
    e = [text(36, 45, "BCore vs Vanilla / NOISE material fill", 27, FG, 650),
         text(36, 74, "Original production kernels · identical block and ordered effect hashes · higher is faster", 13, MUTED),
         rect(36, 91, 11, 11, BLUE, 3, "none"), text(55, 101, "Vanilla 26.1", 12),
         rect(175, 91, 11, 11, TEAL, 3, "none"), text(194, 101, "BCore (release)", 12),
         text(958, 101, "CHUNKS / SECOND", 11, MUTED, 600, "end")]
    for tick in range(5):
        value = max_value * tick / 4
        x = chart_x + chart_w * tick / 4
        e += [line(x, 127, x, 409), text(x, 430, f"{value:g}", 12, MUTED, anchor="middle")]
    description = []
    for i, workers in enumerate(groups):
        y = 139 + i * 94
        e += [text(38, y + 21, f"{workers} worker" + ("s" if workers != 1 else ""), 18, FG, 600),
              text(38, y + 43, "parallelism budget", 11, MUTED)]
        for j, (engine, color) in enumerate([("Vanilla 26.1", BLUE), ("BCore", TEAL)]):
            row = next(r for r in data["summary"] if r["engine"] == engine and r["workers"] == workers)
            value = row["chunks_per_second"]
            low = len(data["chunks"]) / row["max_process_median_seconds"]
            high = len(data["chunks"]) / row["min_process_median_seconds"]
            bar_y = y + j * 32
            length = chart_w * value / max_value
            lo, hi = chart_x + chart_w * low / max_value, chart_x + chart_w * high / max_value
            e += [rect(chart_x, bar_y, round(length, 2), 22, color, 5, "none"),
                  line(lo, bar_y + 11, hi, bar_y + 11, FG, 1.5),
                  line(lo, bar_y + 6, lo, bar_y + 16, FG, 1.5),
                  line(hi, bar_y + 6, hi, bar_y + 16, FG, 1.5),
                  text(max(chart_x + length + 13, hi + 9), bar_y + 16, f"{value:.1f}", 14, color, 650)]
            description.append(f"{engine}, {workers} workers: {value:.1f} chunks/second")
    cpu = data["hardware"].get("cpu", {})
    cpu_name = cpu.get("Name", "CPU details in measurement JSON").strip().replace(" 8-Core Processor", "")
    os_name = data["hardware"]["os"]
    build = re.search(r"Windows-.*?10\.0\.(\d+)", os_name)
    if build and int(build.group(1)) >= 22000:
        os_name = "Windows 11"
    e += [line(30, 452, 970, 452),
          text(38, 479, f'{cpu_name} · {os_name} · {len(data["chunks"])} fresh chunks per batch', 13),
          text(38, 502, f'{data["processes_per_configuration"]} fresh processes × {data["measured_batches_per_process"]} timed batches per configuration; {data["warmup_batches_per_process"]} warmup batches per process.', 12, MUTED),
          text(38, 524, "Bars: median of process medians. Whiskers: process-median range. No concurrent benchmark jobs.", 12, MUTED),
          text(38, 552, "KERNEL SCOPE ONLY — excludes full chunk generation, server startup, I/O, packets and gameplay/TPS.", 12, AMBER, 600)]
    return document(576, "Measured NOISE kernel throughput: BCore versus Vanilla", "; ".join(description) + ". This is not a full-server benchmark.", e)


def pipeline():
    names = [("EMPTY", "ready"), ("STARTS", "partial"), ("REFS", "partial"), ("BIOMES", "ready"),
             ("NOISE", "ready"), ("SURFACE", "ready"), ("CARVERS", "ready"), ("FEATURES", "partial"),
              ("INIT LIGHT", "ready"), ("LIGHT", "ready"), ("SPAWN", "partial"), ("FULL", "next")]
    e = [text(36, 44, "Runtime generation path", 27, FG, 650),
         text(36, 71, "Implementation coverage by stage. Earlier omissions still keep overall requests partial.", 13, MUTED),
         line(52, 120, 948, 120, LINE, 3)]
    for i, (label, status) in enumerate(names):
        x = 52 + i * 81.45
        color = {"ready": TEAL, "partial": AMBER, "next": MUTED}[status]
        e += [f'<circle cx="{x:.2f}" cy="120" r="12" fill="{CARD}" stroke="{color}" stroke-width="2"/>',
              f'<circle cx="{x:.2f}" cy="120" r="4" fill="{color}"/>',
              text(round(x, 2), 153, label, 10, color, 600, "middle")]
    e += [text(36, 191, "● Implemented", 12, TEAL), text(202, 191, "● Partial families / callbacks", 12, AMBER),
          text(474, 191, "● Pending runtime integration", 12, MUTED),
           text(36, 221, "SPAWN runs with explicit inputs; 456 native mob saves verify LOAD, storage and pairing. Live clock hookup remains.", 12, MUTED)]
    return document(246, "BCore runtime generation-stage coverage", "Through SPAWN is integrated with partial coverage. Live clock hookup, FULL conversion and ticket-driven lifecycle remain incomplete.", e)


def import_benchmark(path):
    data = load(path)
    if data.get("verified_matching_outputs") is not True or data["scope"] != "noise-fill-kernel":
        raise ValueError("benchmark must have completed with matching outputs")
    for sample in data["samples"]:
        if sample["fingerprints"] != data["fingerprints"] or not all(t > 0 and math.isfinite(t) for t in sample["seconds"]):
            raise ValueError("invalid benchmark sample")
    for row in data["summary"]:
        samples = [s for s in data["samples"] if (s["engine"], s["workers"]) == (row["engine"], row["workers"])]
        medians = [statistics.median(s["seconds"]) for s in samples]
        if len(samples) != data["processes_per_configuration"] or statistics.median(medians) != row["median_batch_seconds"]:
            raise ValueError("invalid benchmark summary")
    published = {**data, "capture_results_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    with BENCHMARK.open("x", encoding="utf-8") as stream:
        json.dump(published, stream, indent=2)
        stream.write("\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--benchmark", type=Path, help="import completed results.json into a NEW measurement")
    args = parser.parse_args()
    if args.benchmark:
        if args.check:
            parser.error("--check cannot publish measurements")
        import_benchmark(args.benchmark)
    checkpoint = load(CHECKPOINT)
    historical = load(ROOT / "docs/parity-live-generation.json")
    benchmark = load(BENCHMARK)
    files = {"overview.svg": overview(checkpoint), "generation-accuracy.svg": accuracy(checkpoint, historical),
             "regression-fixes.svg": fixes(checkpoint), "performance.svg": performance(benchmark),
             "generation-pipeline.svg": pipeline()}
    for name, contents in files.items():
        ET.fromstring(contents)
        path = ASSETS / name
        if args.check:
            if path.read_text(encoding="utf-8") != contents:
                raise SystemExit(f"Stale graphic: {path.relative_to(ROOT)}")
        else:
            path.write_text(contents, encoding="utf-8", newline="\n")
    print(f'{"Verified" if args.check else "Rendered"} {len(files)} evidence-backed SVGs')


if __name__ == "__main__":
    main()
