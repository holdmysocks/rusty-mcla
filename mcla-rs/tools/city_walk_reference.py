"""The cells' polygon walks on their own, word for word: F-54631 (the capsule's first polygon: F-32549 with the key
0xFFFF, F-94830 -> F-26111 / F-93593 with F-95864, F-88692, F-67335 -> F-81407, F-44462) and F-20956 (the sphere's:
F-32549, F-33149 -> F-95142 / F-54895 with F-33493, F-63013), run offline on the city's collision file laid out as
city_dispatch_reference.py lays it. Out per case: the polygon count F-54631 / F-20956 leave in the query (+56), the
slot, the first polygon's index (+60) and the whole list the walk wrote into the 4096-entry scratch, in its order.

A case: the mode (0 capsule, 1 sphere), the cell's place in the grid's list, the ends (the sphere: the centre and three
unused words), the radius. Writes <out-dir>/city_walk_ref.txt. Test `camprobe::tests::city_walk_matches_the_original`."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import city_dispatch_reference as cd
import recomp_dispatch as rd

MAIN = r'''
static const uint32_t RES=RES_, Q=0x40E00000, VT=0x40E05000, SCR=0x40E10000;
static std::vector<uint8_t> blob;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 {std::ifstream b(argv[3],std::ios::binary); blob.assign(std::istreambuf_iterator<char>(b),{});}
 memcpy(pointer(RES),blob.data(),blob.size());
 uint32_t GRID=strtoul(argv[4],nullptr,16);
 FILE*f=fopen(argv[2],"r"); unsigned w[9];
 while(fscanf(f,"%x %x %x %x %x %x %x %x %x",&w[0],&w[1],&w[2],&w[3],&w[4],&w[5],&w[6],&w[7],&w[8])==9){
  bool ball=w[0]!=0; rt_reset();
  for(uint32_t a=Q;a<VT+0x200;a+=4) store(a,0);
  for(uint32_t a=SCR;a<SCR+8192;a+=4) store(a,0xEEEEEEEE);
  uint32_t it=ball?Q+192:Q+224;
  if(ball){ store(Q+292,SCR); store16(Q+296,4096); for(int j=0;j<3;j++) store(Q+160+4*j,w[2+j]); store(Q+176,w[8]); }
  else { store(Q+324,SCR); store16(Q+328,4096); for(int j=0;j<3;j++){ store(Q+176+4*j,w[2+j]); store(Q+192+4*j,w[5+j]); } store(Q+208,w[8]); }
  uint32_t cell=load(load(GRID+0xa0)+4*w[1]);
  Context c=fresh(); c.r3.u64=Q; c.r4.u64=cell; c.r5.u64=it; c.r6.u64=0xFFFF;
  try{ if(ball) sub_BFIRST_(c,nullptr); else sub_CFIRST_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  uint32_t got=c.r3.u32; int slot=(int)load(it+756); unsigned n=load16(Q+56);
  unsigned start=slot>=0?load16(it+628+2*slot):0, cnt=slot>=0?load16(it+106+2*slot):0;
  printf("%x %x %x %x %x",n,slot,load16(Q+60),got?(got-load(cell+140))/16:0xFFFFFFFFu,cnt);
  for(unsigned k=0;k<cnt;k++) printf(" %x",load16(SCR+2*(start+k)));
  printf("\n"); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--city', default='D:/MCLA/extracted/cache/resources/city/sc/0x3e5fd91d.r1')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    ap.add_argument('--seed', type=int, default=54631)
    a = ap.parse_args()
    blob, grid, cells = cd.load_city(a.city)
    rnd = random.Random(a.seed)
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    f32, fb = cd.f32, cd.fb
    full = [c for c in cells if c[2]]
    cases = []
    while len(cases) < a.cases:
        ball = rnd.random() < 0.5
        li, vx, tris, _ = rnd.choice(full)
        t = rnd.randrange(len(tris))
        on = [sum(vx[tris[t][k]][j] for k in range(3)) / 3 + rnd.gauss(0, 1.0) for j in range(3)]
        R = f32(rnd.choice([0.25, 0.5, 1.0, rnd.uniform(0.02, 3.0)]))
        dv = unit([rnd.gauss(0, 1) for _ in range(3)])
        L = rnd.choice([rnd.uniform(0.0, 1.0), rnd.uniform(1.0, 8.0), rnd.uniform(5.0, 60.0)])
        A = [f32(on[j] - dv[j] * L * 0.5) for j in range(3)]
        B = [f32(A[j] + dv[j] * L) for j in range(3)] if not ball else [0.0, 0.0, 0.0]
        cases.append([int(ball), li] + [fb(v) for v in A + B] + [fb(R)])
    work = a.out_dir / 'build_walk'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'city.bin').write_bytes(blob)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%x' % w for w in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    cfirst, bfirst = cd.addr(a.tool, 'F-54631'), cd.addr(a.tool, 'F-20956')
    main_src = MAIN.replace('RES_', '0x%08Xu' % cd.RES).replace('CFIRST_', cfirst).replace('BFIRST_', bfirst)
    exe = rd.build(idx, ['sub_' + cfirst, 'sub_' + bfirst], main_src, work, 'walk', image=img, limit=400)
    r = cd._run([str(exe), str(a.image), str(inp), str(work / 'city.bin'), '%x' % grid], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    assert not [o for o in out if 'EXC' in o]
    nl = chr(10)
    text = nl.join(' '.join('%x' % w for w in c) + ' -> ' + o.strip() for c, o in zip(cases, out)) + nl
    (a.out_dir / 'city_walk_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', text))
    lens = [int(o.split()[4], 16) for o in out]
    print(len(cases), 'cases;', sum(x > 0 for x in lens), 'with polygons;', max(lens), 'most;', sum(lens), 'in all')


if __name__ == '__main__':
    main()
