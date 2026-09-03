import json, pathlib, sys
ROOT=pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0,str(ROOT/'scripts')); import analyze_terrain_capture as d
cap={}
for _,p in d.packets(d.CAPTURE.read_bytes()):
 c=d.decode(p)
 if (c['x'],c['z'])==(-34,3): cap=c
b=json.loads((pathlib.Path.home()/'AppData/Local/Temp/bcore_chunk.json').read_text())['states']
blocks=json.loads((ROOT/'target/datagen/reports/blocks.json').read_text())
idn={int(s['id']):n for n,e in blocks.items() for s in e.get('states',[])}
v=[]
for sec in cap['sections']: v.extend(sec['states'])
print('lengths',len(v),len(b))
count=0
for i,(a,c) in enumerate(zip(v,b)):
 if idn.get(a)!=idn.get(c):
  sy=i//4096; li=i%4096; y=-64+sy*16+li//256; col=li%256; x=col%16; z=col//16
  print(x,y,z,idn.get(a,a),idn.get(c,c)); count+=1
  if count>=40: break
print('diffs shown',count,'total',sum(idn.get(a)!=idn.get(c) for a,c in zip(v,b)))
