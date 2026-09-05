from __future__ import annotations
import json, pathlib, subprocess, sys
from collections import Counter
ROOT=pathlib.Path(__file__).resolve().parent.parent
CAP=ROOT/'crates/bcore-protocol/data/vanilla_terrain_chunks.bin'
REPORT=ROOT/'target/datagen/reports/blocks.json'
SEED=846692123413862008
MIN_Y=-64
sys.path.insert(0,str(ROOT/'scripts'))
import analyze_terrain_capture as dec

def load_idmap():
    r=json.loads(REPORT.read_text())
    d={}
    for name,e in r.items():
        for s in e.get('states',[]): d[int(s['id'])]=name
    return d

def load_vanilla():
    out={}
    for _,p in dec.packets(CAP.read_bytes()):
        c=dec.decode(p); st=[]
        for sec in c['sections']: st += sec['states']
        out[(c['x'],c['z'])]=st
    return out

def bcore(coords):
    out={}
    for i,(cx,cz) in enumerate(coords,1):
        print(f'BCore {i}/{len(coords)} {cx},{cz}',file=sys.stderr,flush=True)
        p=subprocess.run([str(ROOT/'target/release/examples/dump_chunk.exe'),str(SEED),str(cx),str(cz)],cwd=ROOT,text=True,capture_output=True,check=True)
        out[(cx,cz)]=json.loads(p.stdout)['states']
    return out

def col(st,x,z):
    return [st[y*256+z*16+x] for y in range(384)]

def main():
    names=load_idmap(); van=load_vanilla(); coords=sorted(van)
    bc=bcore(coords)
    fluid={'minecraft:water','minecraft:lava'}
    def nm(i, side):
        if side=='v': return names.get(i,'<unknown>')
        # BCore dump uses canonical vanilla state ids per lib.rs.
        return names.get(i,'<unknown>')
    def top(c,side, solid):
        for i in range(383,-1,-1):
            n=nm(c[i],side)
            if n!='minecraft:air' and (not solid or n not in fluid): return (MIN_Y+i,n)
        return (None,'minecraft:air')
    cats=Counter(); exact_blocks=0; total_blocks=0; total_cols=0; height_ok=0; land_ok=0
    examples={}
    for co in coords:
      v,b=van[co],bc[co]
      for vi,bi in zip(v,b):
        total_blocks+=1
        if nm(vi,'v')==nm(bi,'b'): exact_blocks+=1
      for z in range(16):
       for x in range(16):
        total_cols+=1; vc=col(v,x,z); bb=col(b,x,z)
        vt,vn=top(vc,'v',True); bt,bn=top(bb,'b',True)
        vo,von=top(vc,'v',False); bo,bon=top(bb,'b',False)
        if vt is not None and bt is not None and abs(vt-bt)<=3: height_ok+=1
        vwater=von in fluid; bwater=bon in fluid
        if vwater==bwater: land_ok+=1
        # mutually exclusive diagnostic buckets, with height mismatch first.
        if vt is None or bt is None or abs(vt-bt)>3: cat='height_shift'
        elif (not vwater) and bwater: cat='water_instead_land'
        elif vwater and (not bwater): cat='land_instead_water'
        elif vt is not None and bt is not None and abs(vt-bt)<=3 and vn!=bn: cat='block_type_at_height'
        else: cat='match'
        cats[cat]+=1
        if cat!='match' and cat not in examples and (vt is not None or bt is not None):
            examples[cat]=(co,x,z,(vn,vt),(bn,bt),(von,vo),(bon,bo))
    print(json.dumps({'chunks':len(coords),'columns':total_cols,'blocks':total_blocks,'block_name_exact_pct':100*exact_blocks/total_blocks,'topmost_within_3_pct':100*height_ok/total_cols,'land_water_match_pct':100*land_ok/total_cols,'categories':cats,'examples':examples},indent=2,sort_keys=True))
if __name__=='__main__': main()
