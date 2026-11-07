"""A polyhedron bound's polygon areas as the original recomputes them after a car's hull is reshaped (F-89464 -> F-83274):
for each of the +204 polygons (records +140, 16 bytes: +0 the area, +4 / +6 / +8 the vertex indices) its three vertices
(+128, 16 bytes each) go to F-12307, which writes polygon +0 = 0.5 x |(v[+4] - v[+6]) x (v[+8] - v[+6])|. Run offline on
random polygons (near-degenerate, large, small) and on every vp_* car's hull with its file vertices moved a little.

A case: n (1 .. 64) polygons, then per polygon its three vertices (9 words). Out: each polygon's +0 word.
Writes <out-dir>/poly_area_ref.txt. Test `camprobe::tests::poly_area_matches_the_original`."""
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


def _gpp(cmd, *a, **k):
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


def addr(tool, fid):
    return _run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


MAIN = r'''
static const uint32_t B=0x40D00000, V=0x40D01000, P=0x40D04000, STK=0x40DF0000;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 FILE*f=fopen(argv[2],"r"); unsigned n;
 while(fscanf(f,"%x",&n)==1){
  if(n<1||n>64){fprintf(stderr,"bad case file: %u polygons\n",n); return 2;}
  rt_reset(); for(uint32_t a=B;a<P+0x1000;a+=4) store(a,0);
  store(B+128,V); store(B+140,P); store(B+204,n);
  for(unsigned i=0;i<n;i++){ for(int k=0;k<9;k++){ unsigned w; if(fscanf(f,"%x",&w)!=1) return 1; store(V+48*i+16*(k/3)+4*(k%3),w); }
   store(P+16*i,0xDEADBEEF); store16(P+16*i+4,3*i); store16(P+16*i+6,3*i+1); store16(P+16*i+8,3*i+2); }
  Context c=fresh(); c.r1.u64=STK; c.r3.u64=B;
  try{ sub_F_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  for(unsigned i=0;i<n;i++) printf(i?" %08x":"%08x",load(P+16*i));
  printf("\n"); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, default=Path('D:/MCLA/extracted/cache'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--seed', type=int, default=89464)
    a = ap.parse_args()
    rnd = random.Random(a.seed)
    f32 = lambda x: struct.unpack('>f', struct.pack('>f', x))[0]
    cases = []
    for _ in range(300):
        n = rnd.randint(1, 64)
        tris = []
        for _ in range(n):
            s = rnd.choice([1e-3, 0.1, 1.0, 3.0, 100.0])
            p = [rnd.uniform(-3, 3) for _ in range(3)]
            t = [[f32(p[j] + rnd.gauss(0, s)) for j in range(3)] for _ in range(3)]
            if rnd.random() < 0.1:
                t[2] = [f32(t[0][j] + (t[1][j] - t[0][j]) * 0.5 + rnd.gauss(0, 1e-6)) for j in range(3)]
            tris.append(t)
        cases.append(tris)
    # the cars' hulls: the file's polygons with their vertices moved a little (as a reshape moves them)
    for dr in sorted(glob.glob(str(a.data / 'resources/vehicle/vp*'))):
        for fn in sorted(glob.glob(dr + '/*.r83')):
            raw = Path(fn).read_bytes()
            d = raw[16:16 + struct.unpack_from('<I', raw, 12)[0]]
            u32 = lambda o: struct.unpack_from('>I', d, o)[0]
            F = lambda o: struct.unpack_from('>f', d, o)[0]
            got = None
            for o in range(0, len(d) - 0xe0, 16):
                if d[o + 4] == 4 and struct.unpack_from('<I', d, o)[0] == 0x0059b464:
                    nv, nt = u32(o + 0xc8), u32(o + 0xcc)
                    v, t = u32(o + 0x80) & 0xfffffff, u32(o + 0x8c) & 0xfffffff
                    vs = [[F(v + 16 * i + 4 * j) for j in range(3)] for i in range(nv)]
                    got = [[struct.unpack_from('>H', d, t + 16 * k + 4 + 2 * m)[0] for m in range(3)] for k in range(nt)]
                    break
            if got:
                for rep in range(2):
                    mv = [[f32(x + (rnd.gauss(0, 0.02) if rep else 0.0)) for x in p] for p in vs]
                    for k in range(0, len(got), 64):
                        cases.append([[mv[i] for i in tri] for tri in got[k:k + 64]])
            break
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    assert all(1 <= len(c) <= 64 for c in cases)  # (the reader's layout)
    inp.write_text(''.join('%x ' % len(c) + ' '.join('%x' % fb(x) for t in c for p in t for x in p) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fn = addr(a.tool, 'F-83274')
    subprocess.run = _gpp
    rd.vector_shim = wfr._shim
    try:
        exe = rd.build(idx, ['sub_' + fn], MAIN.replace('sub_F_', 'sub_' + fn), work, 'poly_area', image=img)
    finally:
        subprocess.run = _run
        rd.vector_shim = wfr._vs
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    assert not [o for o in out if 'EXC' in o]
    nl = chr(10)
    text = nl.join('%x ' % len(c) + ' '.join('%x' % fb(x) for t in c for p in t for x in p) + ' -> ' + o.strip() for c, o in zip(cases, out)) + nl
    (a.out_dir / 'poly_area_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', text))
    print(len(cases), 'cases', sum(len(c) for c in cases), 'polygons')


if __name__ == '__main__':
    main()
