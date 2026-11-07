"""The player car's hull as the original reshapes it when a set-up is applied: the point sort F-31335 (into the car's index lists
+680 (4, the floor), +696 / +704 (2 each, the flanks), +712 (2, the bumper bar) and the heights / half width +720 .. +732), then
F-89464 with f1 = how much lower the car sits (moves each listed point by F-46634, then the polygon areas F-83274 and the
bound's own box / sphere, vtable slot 40 F-38448 -> F-90660). Run offline on every vp_* car's hull bound (the file's header,
vertices and polygons) with random set-up values: tune +320 BoundClearance, +324 BoundAngle, +328 BoundBumper, +336
BoundBumperWidth (+332 BoundWidebody only read when the wide body flag is set: left clear here, as the port has no kit).
F-38448's sphere step (its first callee after the box, which works through the thread's allocator) is stubbed: the bound's
radius / centroid words +8 / +12 / +48 .. in the output are not the original's; only the box +16 .. +40 is compared.

A case: nv nt, the header's 56 words, nv x 4 vertex words, nt x 4 polygon words, the 4 tune words, the drop word.
Out: the car's +680 .. +732 (14 words), each vertex's x y z, the header's +8 .. +60 (14 words).
Writes <out-dir>/hull_reshape_ref.txt. Test `hull::tests::reshape_matches_the_original`."""
import argparse
import glob
import random
import re
import struct
import subprocess
from pathlib import Path
import memcap  # (every harness process capped at 4 GB)
import recomp_dispatch as rd
import wheel_fit_reference as wfr  # (its vector shim: the unaligned loads' right-hand mask table)

_run = subprocess.run
VT_POLY = 0x82081784   # the polyhedron bound's vtable (slot 40 F-38448)


def _gpp(cmd, *a, **k):
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


def addr(tool, fid):
    return _run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


MAIN = r'''
static const uint32_t CAR=0x40D00000, A=0x40D01000, T=0x40D03000, B=0x40D04000, V=0x40D05000, P=0x40D06000, STK=0x40DF0000;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 FILE*f=fopen(argv[2],"r"); unsigned nv,nt;
 while(fscanf(f,"%x %x",&nv,&nt)==2){
  if(nv<4||nv>64||nt<1||nt>128){fprintf(stderr,"bad case file: %u vertices %u polygons\n",nv,nt); return 2;}
  rt_reset(); for(uint32_t a=CAR;a<P+0x1000;a+=4) store(a,0);
  unsigned w;
  for(int k=0;k<56;k++){ if(fscanf(f,"%x",&w)!=1) return 1; store(B+4*k,w); }
  store(B+0,0x82081784u); store(B+128,V); store(B+132,0); store(B+136,0); store(B+140,P); store(B+176,0); store(B+180,0);
  store(B+208,0); store(B+216,0); store(B+200,nv); store(B+204,nt);
  for(unsigned i=0;i<4*nv;i++){ if(fscanf(f,"%x",&w)!=1) return 1; store(V+4*i,w); }
  for(unsigned i=0;i<4*nt;i++){ if(fscanf(f,"%x",&w)!=1) return 1; store(P+4*i,w); }
  unsigned tw[4], drop; for(int k=0;k<4;k++){ if(fscanf(f,"%x",&tw[k])!=1) return 1; } if(fscanf(f,"%x",&drop)!=1) return 1;
  store(CAR+24,4); store(CAR+4,A); store(CAR+368,T); for(int k=0;k<10;k++) store(CAR+680+4*k,0xFFFFFFFFu);
  store(A+28,A+0x100); store(A+0x100+4,A+0x200); store(A+0x200+12,A+0x300); store(A+0x300+128,A+0x400); store(A+0x400,B);
  store(A+132,A+0x800);
  store(T+320,tw[0]); store(T+324,tw[1]); store(T+328,tw[2]); store(T+336,tw[3]);
  int stage=0;
  try{ Context c=fresh(); c.r1.u64=STK; c.r3.u64=CAR; sub_SORT_(c,nullptr); stage=1;
       Context d=fresh(); d.r1.u64=STK; d.r3.u64=CAR; uint32_t dw=drop; float df; memcpy(&df,&dw,4); d.f1.f64=df; sub_MOVE_(d,nullptr); }
  catch(std::exception&e){ printf("EXC %s stage %d\n",e.what(),stage); continue; }
  for(int k=0;k<14;k++) printf(k?" %08x":"%08x",load(CAR+680+4*k));
  for(unsigned i=0;i<nv;i++) for(int k=0;k<3;k++) printf(" %08x",load(V+16*i+4*k));
  for(int k=2;k<16;k++) printf(" %08x",load(B+4*k));
  printf("\n"); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, default=Path('D:/MCLA/extracted/cache'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--seed', type=int, default=31335)
    ap.add_argument('--per-car', type=int, default=10)
    a = ap.parse_args()
    rnd = random.Random(a.seed)
    cases = []
    for dr in sorted(glob.glob(str(a.data / 'resources/vehicle/vp*'))):
        for fn in sorted(glob.glob(dr + '/*.r83')):
            raw = Path(fn).read_bytes()
            d = raw[16:16 + struct.unpack_from('<I', raw, 12)[0]]
            u32 = lambda o: struct.unpack_from('>I', d, o)[0]
            for o in range(0, len(d) - 0xe0, 16):
                if d[o + 4] == 4 and struct.unpack_from('<I', d, o)[0] == 0x0059b464:
                    nv, nt = u32(o + 200), u32(o + 204)
                    v, t = u32(o + 128) & 0xfffffff, u32(o + 140) & 0xfffffff
                    hdr = [u32(o + 4 * k) for k in range(56)]
                    vs = [u32(v + 4 * i) for i in range(4 * nv)]
                    ps = [u32(t + 4 * i) for i in range(4 * nt)]
                    for _ in range(a.per_car):
                        clear = rnd.uniform(0.05, 0.35) if rnd.random() < 0.85 else rnd.uniform(0.35, 1.5)
                        ang = rnd.uniform(0.0, 0.8) if rnd.random() < 0.9 else 0.0
                        bump = 0.0 if rnd.random() < 0.3 else rnd.uniform(0.2, 0.7)
                        width = rnd.uniform(0.3, 1.0)
                        drop = 0.0 if rnd.random() < 0.5 else rnd.uniform(0.0, 0.15)
                        cases.append((Path(dr).name, nv, nt, hdr, vs, ps, [fb(clear), fb(ang), fb(bump), fb(width)], fb(drop)))
                    break
            break
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    assert all(4 <= c[1] <= 64 and 1 <= c[2] <= 128 and len(c[3]) == 56 and len(c[4]) == 4 * c[1] and len(c[5]) == 4 * c[2]
               for c in cases)  # (the reader's layout)
    line = lambda c: ' '.join('%x' % x for x in [c[1], c[2]] + c[3] + c[4] + c[5] + c[6] + [c[7]])
    inp.write_text(''.join(line(c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    sort, move = addr(a.tool, 'F-31335'), addr(a.tool, 'F-89464')
    subprocess.run = _gpp
    rd.vector_shim = wfr._shim
    try:
        src = MAIN.replace('sub_SORT_', 'sub_' + sort).replace('sub_MOVE_', 'sub_' + move)
        exe = rd.build(idx, ['sub_' + sort, 'sub_' + move], src, work, 'hull_reshape', image=img, vtables=[(VT_POLY, [40])], stubs={'sub_825CA278': ''})
    finally:
        subprocess.run = _run
        rd.vector_shim = wfr._vs
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    nl = chr(10)
    # per case: the car's name, nv, the file's vertices (x y z), the tune words, the drop -> the original's words
    text = nl.join('%s %x ' % (c[0], c[1]) + ' '.join('%x' % c[4][4 * i + k] for i in range(c[1]) for k in range(3)) + ' ' +
                   ' '.join('%x' % x for x in c[6] + [c[7]]) + ' -> ' + o.strip() for c, o in zip(cases, out)) + nl
    (a.out_dir / 'hull_reshape_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', text))
    print(len(cases), 'cases')


if __name__ == '__main__':
    main()
