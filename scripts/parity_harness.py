#!/usr/bin/env python3
"""Independent BCore/vanilla 26.1 parity measurement.

The vanilla probe is always the opped user ``bot`` and dump_terrain.js verifies
its post-/tp position before emitting data.  No worldgen code is changed here.
"""
from __future__ import annotations
import argparse, json, re, subprocess, sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BOT = ROOT / "scripts" / "bot" / "dump_terrain.js"
BCORE = ROOT / "target" / "release" / "examples" / "surface_grid.exe"
CHUNK = ROOT / "target" / "release" / "examples" / "dump_chunk.exe"
SEED = 846692123413862008
SAMPLES = [(0, 0), (1000, 0), (-2000, 3000)]
# Network state IDs used by bcore-worldgen -> human names, generated from
# minecraft-data 26.1 (scripts/block_ids.json).  The earlier hand-written
# 23-entry stub meant every other BCore block fell through to a raw id string
# and could never match the vanilla name, understating top-block parity.
_IDS_PATH = Path(__file__).resolve().parents[1] / "scripts" / "block_ids.json"
IDS = {int(k): v for k, v in json.loads(_IDS_PATH.read_text()).items()}
LOGS = {"oak_log", "birch_log", "spruce_log", "jungle_log", "acacia_log", "dark_oak_log"}

def run(cmd, cwd=ROOT, timeout=240):
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, errors="replace", timeout=timeout)
    if p.returncode:
        raise RuntimeError(f"command failed ({p.returncode}): {' '.join(map(str,cmd))}\n{p.stderr[-2000:]}")
    return p.stdout, p.stderr

def parse_grid(text):
    rows=[]
    for line in text.splitlines():
        cells=[]
        for cell in line.split():
            m=re.fullmatch(r"(-?\d+):(.+)", cell)
            if m: cells.append((int(m.group(1)),m.group(2)))
        if cells: rows.append(cells)
    return rows

def vanilla_grid(x,z,region,ground):
    args=["node",str(BOT),"127.0.0.1","25571",str(x),str(z),str(region),"bot"]
    if ground: args.append("--ground")
    out,err=run(args, cwd=BOT.parent)
    if "position after teleport" not in err:
        raise RuntimeError("vanilla probe did not report verified position")
    return parse_grid(out)

def bcore_grid(x,z,region,ground):
    args=[str(BCORE),str(x),str(z),str(region)]
    if ground: args.append("--ground")
    return parse_grid(run(args)[0])

def bcore_blocks(x,z,region,ymin,ymax):
    out={}
    half=region//2
    for cx in range((x-half)//16, (x+half-1)//16+1):
      for cz in range((z-half)//16, (z+half-1)//16+1):
        raw=json.loads(run([str(CHUNK),str(SEED),str(cx),str(cz)])[0])
        states=raw["states"]
        for wx in range(max(x-half,cx*16), min(x+half,(cx+1)*16)):
          for wz in range(max(z-half,cz*16), min(z+half,(cz+1)*16)):
            lx,lz=wx-cx*16,wz-cz*16
            for y in range(ymin,ymax+1):
                i=(y+64)*256+lz*16+lx
                out[(wx,y,wz)]=IDS.get(states[i],f"state_{states[i]}")
    return out

def block_map_vanilla(x,z,region,ymin,ymax):
    # The probe's own hardcoded ceiling silently capped this to 40..120 before;
    # the range must be passed through or the two sides cover different Y spans.
    args=["node",str(BOT),"127.0.0.1","25571",str(x),str(z),str(region),"bot","--full",f"--ymin={ymin}",f"--ymax={ymax}"]
    out,err=run(args,cwd=BOT.parent)
    if "position after teleport" not in err: raise RuntimeError("vanilla full probe position was not verified")
    data=json.loads(out)
    return {(a,b,c):d for a,b,c,d in data["blocks"]}, data["position"]

def origins(blocks):
    # A tree-origin proxy: lowest log in each vertical trunk at a sampled x,z.
    return {(x,y,z) for (x,y,z),name in blocks.items() if name in LOGS and blocks.get((x,y-1,z)) not in LOGS}

def pct(a,b): return 100.0*a/b if b else 0.0

def main():
    ap=argparse.ArgumentParser(); ap.add_argument("--region",type=int,default=16); ap.add_argument("--no-build-check",action="store_true")
    # Y range must cover the whole column where vegetation can sit: at (0,0) the
    # ground is ~126 and oak foliage reaches ~137, so the old 40..120 ceiling
    # clipped every tree there and reported 0 origins on BOTH sides.
    ns=ap.parse_args(); region=ns.region; ymin,ymax=40,220
    if not BCORE.exists() or not CHUNK.exists(): raise SystemExit("missing release examples; build surface_grid and dump_chunk first")
    records=[]
    for x,z in SAMPLES:
        vg=vanilla_grid(x,z,region,False); vground=vanilla_grid(x,z,region,True)
        bg=bcore_grid(x,z,region,False); bground=bcore_grid(x,z,region,True)
        n=min(len(vg),len(bg))*min(len(vg[0]),len(bg[0]))
        hmatch=sum(vground[r][c][0]==bground[r][c][0] for r in range(min(len(vground),len(bground))) for c in range(min(len(vground[r]),len(bground[r]))) )
        tmatch=sum(vg[r][c][0]==bg[r][c][0] for r in range(min(len(vg),len(bg))) for c in range(min(len(vg[r]),len(bg[r]))) )
        topmatch=sum(vg[r][c][1]==IDS.get(int(bg[r][c][1]),bg[r][c][1]) for r in range(min(len(vg),len(bg))) for c in range(min(len(vg[r]),len(bg[r]))) )
        vb,vpos=block_map_vanilla(x,z,region,ymin,ymax); bb=bcore_blocks(x,z,region,ymin,ymax)
        keys=set(vb)&set(bb); diffs=[k for k in keys if vb[k]!=bb[k]]
        vo,bo=origins(vb),origins(bb); omatch=len(vo&bo)
        records.append(dict(x=x,z=z,columns=n,height_match=hmatch,height_pct=pct(hmatch,n),top_height_match=tmatch,top_height_pct=pct(tmatch,n),top_block_match=topmatch,top_block_pct=pct(topmatch,n),tree_vanilla=len(vo),tree_bcore=len(bo),tree_intersection=omatch,tree_match_pct=pct(omatch,max(len(vo),len(bo))),block_cells=len(keys),block_diff=len(diffs),block_same=len(keys)-len(diffs),position=vpos))
    report=ROOT/"docs"/"parity-report.md"; report.parent.mkdir(exist_ok=True)
    lines=["# BCore / Vanilla 26.1 parity report", "", f"- Seed: `{SEED}`", f"- Servers: BCore `127.0.0.1:25565` (current running build); vanilla `127.0.0.1:25571`", f"- Samples: `{SAMPLES}`; region: `{region}x{region}`", "- Status: numbers below are confirmed by this real run. BCore was not rebuilt by this harness; it measures the currently running/current release artifacts.", "", "## Terrain height parity", "", "| sample | columns | exact height | match |", "|---|---:|---:|---:|"]
    for r in records: lines.append(f"| ({r['x']},{r['z']}) | {r['columns']} | {r['height_match']} | {r['height_pct']:.2f}% |")
    lines += ["", "## Vegetation / surface parity", "", "Top height is the non-air surface and is separated from terrain height; top-block compares block names at that surface. Tree-origin is the exact intersection of lowest sampled log positions.", "", "| sample | top height | top block | vanilla origins | BCore origins | origin match |", "|---|---:|---:|---:|---:|---:|"]
    for r in records: lines.append(f"| ({r['x']},{r['z']}) | {r['top_height_pct']:.2f}% | {r['top_block_pct']:.2f}% | {r['tree_vanilla']} | {r['tree_bcore']} | {r['tree_match_pct']:.2f}% |")
    lines += ["", "## Block-level diff (Y=40..120)", "", "| sample | compared cells | equal | different | diff % |", "|---|---:|---:|---:|---:|"]
    for r in records: lines.append(f"| ({r['x']},{r['z']}) | {r['block_cells']} | {r['block_same']} | {r['block_diff']} | {pct(r['block_diff'],r['block_cells']):.2f}% |")
    report.write_text("\n".join(lines)+"\n",encoding="utf-8")
    print("Confirmed real-run parity measurements:")
    for r in records: print(f"({r['x']},{r['z']}): terrain={r['height_pct']:.2f}% top-height={r['top_height_pct']:.2f}% top-block={r['top_block_pct']:.2f}% tree-origin={r['tree_match_pct']:.2f}% block-diff={r['block_diff']}/{r['block_cells']}")
    print(f"report: {report}")

if __name__=="__main__": main()
