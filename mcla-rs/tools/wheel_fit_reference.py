"""The wheels' fit F-55382 run whole offline on a real car's composite bound, with set-up tyres: the first wheel part's search
(+392: the last part whose current place is within 0.1 (squared) of wheel 0's place of rest +384), every wheel part's resize
(the curved bound's vtable slot 46 F-96106 -> the corners, F-17852 per edge, F-22892 per face, slot 40 F-63967 the bound's
own box by its support F-20142), the tail (each wheel part's current place put at rest y - max(SuspensionExtent, 0.5), its
last matrix +136 the same rows and x / z with y = (SuspensionLimit + rest y) + 0.1), F-65996 (the parts' boxes into +140,
F-32566 the union swept from the last matrices to the current ones, the centroid, the radii) and the places put back. The
level's update of the instance's radius is skipped (the level global 0 in the image, as offline); the composite cast
F-75805 answers the composite. The vector constants are made by the original's static initialisers (gjk_reference.py's ten).

The cars: every vp_* vehicle's first .r83 with a composite bound (type 12: a hull, type 4, and 2 or 4 curved wheel bounds,
type 5) - its header, part list, current / last matrices, part boxes, the hull's header and each wheel bound's header,
corners, curved edges and faces, laid in the arena. Each case runs the fit twice, as a car is made: first with the size
every wheel bound is made with (TireRadius 0.3, TireWidth 0.1), then with the case's tyres (a set-up's:
`simtune::wheel_radius`), each wheel's own SuspensionExtent / SuspensionLimit (either side of 0.5) and place of rest.
The wheel parts' current matrices before the first fit: the file's rows with the place of rest's x / z and the file's y
(most cases), or turned rows (a fifth, as a motorcycle's raked fork has).

A case: the car's index, the wheel count, then 4 wheels x (radius, width, extent, limit, rest x y z, the current matrix's
16 words). Out: car +392, +544, the composite's byte +5, +8, +12, +16 .. +60, then per part its current matrix (16 words),
last matrix (16), local box (8), and for a wheel part its own +8, +12, +16 .. +60. Writes <out-dir>/wheel_fit_ref.txt.
Test `vehsim::tests::wheel_fit_matches_the_original`."""
import argparse
import glob
import math
import os
import random
import re
import struct
import subprocess
from pathlib import Path
import memcap  # (every harness process capped at 4 GB)
import recomp_dispatch as rd

_run = subprocess.run
_vs = rd.vector_shim
SDK_INTRINSICS = 'D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h'


def _shim():
    header = Path(SDK_INTRINSICS).read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    s = '#include <simde/x86/avx2.h>' + chr(10) + _vs()
    old = 'struct Condition {' + chr(10) + '    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    s = s.replace(old, 'struct Condition {' + chr(10) + '    bool so{}, eq{}, lt{}, gt{};' + chr(10) +
                  '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }' + chr(10) +
                  '    void setFromMask(simde__m128i mask, int imm) { int m = simde_mm_movemask_epi8(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)
    return s + mask.replace('inline uint8_t', 'alignas(16) static uint8_t') + chr(10)


def _gpp(cmd, *a, **k):
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


import recomp_closure
recomp_closure.EXTRA += (
    'namespace rex { namespace ppc {' + chr(10) +
    'inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {' + chr(10) +
    '  simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));' + chr(10) +
    '  simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8);' + chr(10) +
    '  xmm0 = simde_mm_srli_epi32(xmm0, 31);' + chr(10) +
    '  xmm0 = simde_mm_add_epi32(xmm0, xmm1);' + chr(10) +
    '  xmm0 = simde_mm_srai_epi32(xmm0, 8);' + chr(10) +
    '  xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000));' + chr(10) +
    '  simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);' + chr(10) +
    '  return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1));' + chr(10) +
    '} } }' + chr(10))
INITS = ['F-50173', 'F-33402', 'F-26199', 'F-55749', 'F-97349', 'F-70580', 'F-79721', 'F-64228', 'F-24192', 'F-92824']
VT_CURVED = 0x82083AA4        # the curved bound's vtable (slot 40 F-63967, slot 46 F-96106)
BLOB, BLOB_SIZE = 0x40D10000, 0x3000
N_WHEEL = 7 + 16
MAIN = r'''
static const uint32_t CAR=0x40D00000, A=0x40D00400, WARR=0x40D00900, WH=0x40D02000, TU=0x40D04000, BL=BLOB_, STK=0x40DF0000;
static std::vector<uint8_t> blobs;
static void pf(uint32_t a,float v){ uint32_t b; memcpy(&b,&v,4); store(a,b); }
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 {std::ifstream b(argv[3],std::ios::binary); blobs.assign(std::istreambuf_iterator<char>(b),{});}
 unsigned ncar=blobs.size()/BLOBSIZE_;
 rt_reset();
 { INITCALLS }
 if(load(0x82860950)!=0xffffffffu || load(0x828609E0)!=0x7F800000u) { printf("EXC constants\n"); return 1; }
 FILE*f=fopen(argv[2],"r"); unsigned h[2];
 while(fscanf(f,"%x %x",&h[0],&h[1])==2){
  if(h[0]>=ncar||(h[1]!=2&&h[1]!=4)){fprintf(stderr,"bad case file: car %u wheels %u\n",h[0],h[1]); return 2;}
  unsigned w[4*NW_]; for(int i=0;i<4*NW_;i++) if(fscanf(f,"%x",&w[i])!=1) return 1;  // (4 wheels x NW_ words, as the writer puts them)
  for(uint32_t a=CAR;a<TU+0x1000;a+=4) store(a,0);
  memcpy(pointer(BL),blobs.data()+size_t(h[0])*BLOBSIZE_,BLOBSIZE_);
  uint32_t comp=BL, n=load16(comp+146);
  if(n<3||n>5){fprintf(stderr,"bad blob: %u parts\n",n); return 2;}
  store(CAR+4,A); store(A+28,A+0x40); store(A+0x40+4,A+0x80); store(A+0x80+12,comp);
  store(CAR+24,h[1]); store(CAR+28,WARR); store(CAR+392,0xFFFFFFFFu); store(CAR+412,WH);
  for(unsigned i=0;i<4;i++){ uint32_t wh=WH+0x400*i, tu=TU+0x100*i; const unsigned*x=w+NW_*i;
   store(WARR+4*i,wh); store(wh+4,tu); pf(tu+4,0.3f); pf(tu+8,0.1f); store(tu+88,x[2]); store(tu+84,x[3]);
   for(int j=0;j<3;j++) store(wh+384+4*j,x[4+j]);
   if(i+1<n) for(int k=0;k<16;k++) store(load(comp+132)+64*(i+1)+4*k,x[7+k]); }
  try{
   { Context c=fresh(); c.r1.u64=STK; c.r3.u64=CAR; sub_FIT_(c,nullptr); }
   for(unsigned i=0;i<4;i++){ uint32_t tu=TU+0x100*i; store(tu+4,w[NW_*i]); store(tu+8,w[NW_*i+1]); }
   { Context c=fresh(); c.r1.u64=STK; c.r3.u64=CAR; sub_FIT_(c,nullptr); }
  }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  printf("%08x %08x %02x",load(CAR+392),load(CAR+544),*pointer(comp+5));
  for(int o=8;o<64;o+=4) if(o<16||o>=16) printf(" %08x",load(comp+o));
  for(unsigned p=0;p<n;p++){ printf(" |");
   for(int k=0;k<16;k++) printf(" %08x",load(load(comp+132)+64*p+4*k));
   for(int k=0;k<16;k++) printf(" %08x",load(load(comp+136)+64*p+4*k));
   for(int k=0;k<8;k++) printf(" %08x",load(load(comp+140)+32*p+4*k));
   uint32_t b=load(load(comp+128)+4*p);
   if(*pointer(b+4)==5) for(int o=8;o<64;o+=4) printf(" %08x",load(b+o)); }
  printf("\n"); } }
'''


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def addr(tool, fid):
    return _run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def car_blob(path):
    """The composite of a car file laid at BLOB as the fit reads it, or None: (blob, the parts' types, the file's part places,
    the hull's box (max, min))."""
    raw = Path(path).read_bytes()
    assert raw[:4] == b'MCLR'
    size = struct.unpack_from('<I', raw, 12)[0]
    d = raw[16:16 + size]
    u32 = lambda a: struct.unpack_from('>I', d, a)[0]
    u16 = lambda a: struct.unpack_from('>H', d, a)[0]
    at = lambda a: u32(a) & 0xfffffff if u32(a) >> 28 == 5 else None
    for o in range(0, len(d) - 0xa0, 16):
        if d[o + 4] != 12 or not 0 < u16(o + 0x92) <= 5 or at(o + 0x80) is None:
            continue
        n = u16(o + 0x92)
        parts = [at(at(o + 0x80) + 4 * i) for i in range(n)]
        if None in parts or [d[p + 4] for p in parts] not in ([4, 5, 5, 5, 5], [4, 5, 5]):
            continue
        mats, last, local = at(o + 0x84), at(o + 0x88), at(o + 0x8c)
        if None in (mats, last, local):
            continue
        b = bytearray(BLOB_SIZE)
        P, M, L, B, H = 0x100, 0x200, 0x400, 0x600, 0x800
        b[0:0xa0] = d[o:o + 0xa0]
        put = lambda off, v: b.__setitem__(slice(off, off + 4), struct.pack('>I', v))
        for off, v in ((0x80, BLOB + P), (0x84, BLOB + M), (0x88, BLOB + L), (0x8c, BLOB + B)):
            put(off, v)
        b[M:M + 64 * n] = d[mats:mats + 64 * n]
        b[L:L + 64 * n] = d[last:last + 64 * n]
        b[B:B + 32 * n] = d[local:local + 32 * n]
        b[H:H + 0xf0] = d[parts[0]:parts[0] + 0xf0]
        for off in range(0x80, 0xf0, 4):          # (the hull's own lists are not read by the fit)
            put(H + off, 0) if b[H + off] >> 4 in (5, 6) else None
        put(P, BLOB + H)
        for k, p in enumerate(parts[1:]):
            W = 0x1000 + 0x800 * k
            assert u32(p + 0xC8) == 4 and u32(p + 0xE8) == 4 and u32(p + 0xEC) == 6, 'not the curved wheel bound'
            v, fc, e = at(p + 0x80), at(p + 0xE0), at(p + 0xE4)
            b[W:W + 0xf0] = d[p:p + 0xf0]
            b[W + 0x100:W + 0x140] = d[v:v + 64]
            b[W + 0x200:W + 0x320] = d[e:e + 288]
            b[W + 0x400:W + 0x580] = d[fc:fc + 384]
            put(W, VT_CURVED)
            put(W + 128, BLOB + W + 0x100)
            put(W + 132, 0)
            put(W + 224, BLOB + W + 0x400)
            put(W + 228, BLOB + W + 0x200)
            put(P + 4 * (k + 1), BLOB + W)
        places = [[struct.unpack_from('>f', d, mats + 64 * i + 48 + 4 * j)[0] for j in range(3)] for i in range(n)]
        assert len(b) == BLOB_SIZE
        return bytes(b), n, places
    return None


RIM = [0x3ea90ffa, 0x3eb61134, 0x3ec3126f, 0x3ed013a9, 0x3edd14e4, 0x3eea161f, 0x3ef71759, 0x3f020c4a, 0x3f088ce7, 0x3f0f0d84,
       0x3f158e21, 0x3f1c0ebe, 0x3f228f5c, 0x3f290ff9, 0x3f2f9096, 0x3f361134, 0x3f3c91d1]


def tyre(rnd):
    """A tyre a set-up gives (simtune::wheel_radius): (radius, width)."""
    w = f32(0.195 + 0.01 * rnd.randrange(17))
    p = f32(0.20 + 0.05 * rnd.randrange(14))
    t = f32(p * w - struct.unpack('>f', struct.pack('>I', 0x3d020c4a))[0])
    r = f32(struct.unpack('>f', struct.pack('>I', RIM[rnd.randrange(17)]))[0] * 0.5 + t)
    return r, w


def turned(rnd, k):
    """Rows turned about x (a rake) by up to k radians and about y a little."""
    a, s = rnd.uniform(-k, k), rnd.uniform(-k / 3, k / 3)
    ca, sa, cs, ss = math.cos(a), math.sin(a), math.cos(s), math.sin(s)
    rx = [[1, 0, 0], [0, ca, sa], [0, -sa, ca]]
    ry = [[cs, 0, -ss], [0, 1, 0], [ss, 0, cs]]
    return [[f32(sum(rx[i][m] * ry[m][j] for m in range(3))) for j in range(3)] for i in range(3)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, default=Path('D:/MCLA/extracted/cache'))
    ap.add_argument('--sdk', default=SDK_INTRINSICS)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--per-car', type=int, default=8)
    ap.add_argument('--seed', type=int, default=55382)
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    cars = []
    for dr in sorted(glob.glob(str(a.data / 'resources/vehicle/vp*'))):
        for f in sorted(glob.glob(dr + '/*.r83')):
            c = car_blob(f)
            if c:
                cars.append((os.path.basename(dr), c))
                break
    assert cars, 'no car files'
    rnd = random.Random(a.seed)
    cases = []
    for ci, (name, (blob, n, places)) in enumerate(cars):
        for k in range(a.per_car):
            nw = n - 1
            same = rnd.random() < 0.1
            tyres = [tyre(rnd) if not same else (0.3, 0.1) for _ in range(2)]
            ws = []
            for i in range(4):
                fr = i < 2 if nw == 4 else i == 0
                r, wd = tyres[0 if fr else 1]
                ext = f32(rnd.choice([0.1, 0.2, 0.3, 0.5, 0.6, rnd.uniform(0.05, 0.8)]))
                lim = f32(rnd.choice([0.1, 0.15, 0.2, rnd.uniform(0.05, 0.4)]))
                pl = places[1 + i] if i < nw else [0.0, 0.0, 0.0]
                rest = [f32(pl[0] + (rnd.gauss(0, 0.01) if k % 3 == 2 else 0.0)), f32(pl[1] - rnd.uniform(0.0, 0.05)), f32(pl[2] + (rnd.gauss(0, 0.01) if k % 3 == 2 else 0.0))]
                rows = turned(rnd, 0.5) if (k % 5 == 4 and (i == 0 or rnd.random() < 0.3)) else [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
                mat = [x for r_ in rows for x in r_ + [0.0]] + [rest[0], pl[1], rest[2], 1.0]
                ws += [fb(r), fb(wd), fb(ext), fb(lim)] + [fb(x) for x in rest] + [fb(f32(x)) for x in mat]
            assert len(ws) == 4 * N_WHEEL  # (the reader's layout)
            cases.append([ci, nw] + ws)
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'cars.bin').write_bytes(b''.join(c[1][0] for c in cars))
    (work / 'cars.txt').write_text(''.join('%d %s\n' % (i, c[0]) for i, c in enumerate(cars)))
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%x' % w for w in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fit = addr(a.tool, 'F-55382')
    cast = addr(a.tool, 'F-75805')
    inits = [addr(a.tool, f) for f in INITS]
    main_src = (MAIN.replace('BLOB_', '0x%08Xu' % BLOB).replace('BLOBSIZE_', str(BLOB_SIZE)).replace('NW_', str(N_WHEEL))
                .replace('FIT_', fit).replace('INITCALLS', ' '.join('{Context g=fresh(); sub_%s(g,nullptr);}' % x for x in inits)))
    rd.vector_shim = _shim
    subprocess.run = _gpp
    try:
        exe = rd.build(idx, ['sub_' + fit] + ['sub_' + x for x in inits], main_src, work, 'wheel_fit', image=img,
                       stubs={'sub_' + cast: ''}, vtables=[(VT_CURVED, [40, 46])])
    finally:
        subprocess.run = _run
        rd.vector_shim = _vs
    r = _run([str(exe), str(a.image), str(inp), str(work / 'cars.bin')], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stdout[-300:], r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    nl = chr(10)
    text = nl.join(' '.join('%x' % w for w in c) + ' -> ' + o.strip() for c, o in zip(cases, out)) + nl
    (a.out_dir / 'wheel_fit_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', text))
    print(len(cars), 'cars;', len(cases), 'cases')


if __name__ == '__main__':
    main()
