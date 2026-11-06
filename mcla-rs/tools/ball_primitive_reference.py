"""F-15615: a ball (the query's centre +160 in the bound's frame, radius +176) against a bound that is not made of
polygons - a sphere (type 0: centroid +48, radius +8), a capsule (type 1: radius +128, length +144 along y) or a box
(type 3: size +208, with the box's feature classifier F-25936) - run offline on the original's code whole; only the
result list's alloc F-89028 is stubbed (it logs the depth f1, the cap f2 and the point r5 points at, and answers a record
when the case says so and the depth is above 0, as the real one does) and the bound's material call faked (it answers
(index + 1) << 8, the index it is asked for: the part the record names). The curved geometry (type 5) is not run (its
helpers throw if reached).

A case: the type, the bound's centroid, radius, capsule radius and length, box size; the ball's centre and radius;
the material mask +88; whether alloc gives a record. Boxes: centres inside (by each face), over a face, by an edge, by a
corner, far, exactly on an edge's line or a corner; capsules and spheres about their surface and on the axis.

Out per case: the result r3, alloc called, f1, f2, the point, the record (+0, +16 .. +24, +32 .. +40, +48 .. +56, +64,
+68, +72, +76, +78, +80, +82). Writes <out-dir>/ball_primitive_ref.txt. Test
`camprobe::tests::ball_primitive_matches_the_original`."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


import recomp_closure
recomp_closure.EXTRA += 'static int GIVE_;' + chr(10)
_vs = rd.vector_shim
_run = subprocess.run


def _shim():
    # (the ball's radius is splatted from an unaligned load: the right-hand mask table of the recomp's intrinsics)
    header = Path(SDK_INTRINSICS).read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    return _vs() + mask.replace('inline uint8_t', 'alignas(16) static uint8_t') + chr(10)


def _gpp(cmd, *a, **k):
    # the recomp's own x86 flag: vmsum3fp is dpps, (lanes 0 + 1) + (2 + 3)
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


rd.vector_shim = _shim
subprocess.run = _gpp
SDK_INTRINSICS = 'D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h'
FAKE_MAT = 0x7F000010
CURVED = ['F-80030', 'F-96503', 'F-48308']   # the curved geometry's helpers (type 5: not run here)
N_IN = 1 + 3 + 1 + 1 + 1 + 3 + 3 + 1 + 1 + 1
MAIN = r'''
static const uint32_t Q=0x40800000, BND=0x40801000, VT=0x40801400, REC=0x40802400;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}

int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 for(int k=0;k<4;k++) put(0x828609E0+4*k,0x7F800000);
 FILE*f=fopen(argv[2],"r"); unsigned w[NIN_];
 while(1){ int n=0; for(int i=0;i<NIN_;i++) n+=fscanf(f,"%x",&w[i]); if(n!=NIN_)break;
  rt_reset(); for(uint32_t a=Q;a<REC+0x100;a+=4) store(a,0);
  store(BND,VT); store(VT+48,FAKE_MAT_); *pointer(BND+4)=uint8_t(w[0]);
  for(int j=0;j<3;j++) store(BND+48+4*j,w[1+j]);
  store(BND+8,w[4]); store(BND+128,w[5]); store(BND+144,w[6]);
  for(int j=0;j<3;j++) store(BND+208+4*j,w[7+j]);
  for(int j=0;j<3;j++) store(Q+160+4*j,w[10+j]);
  store(Q+176,w[13]); store(Q+88,w[14]);
  GIVE_=int(w[15]);
  for(uint32_t a=REC;a<REC+96;a+=4) store(a,0xDEADBEEF);
  store(0x40802300u,0); store(0x40802304u,0);
  Context c=fresh(); c.r3.u64=Q; c.r4.u64=BND; c.r5.u64=0x1234;
  try{ sub_BALL_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  int called=load(0x40802304u); bool rec=called&&GIVE_&&load(0x40802320u);
  printf("%d %d %08x %08x %08x %08x %08x",c.r3.s32&0xFF,called,load(0x40802300u),load(0x40802308u),load(0x40802310u),load(0x40802314u),load(0x40802318u));
  for(int o: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",rec?load(REC+o):0u);
  printf(" %04x %04x %02x %04x\n", rec?load16(REC+76):0u, rec?load16(REC+78):0u, rec?*pointer(REC+80):0u, rec?load16(REC+82):0u); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--sdk', default=SDK_INTRINSICS, help='the recomp SDK intrinsics.h (VectorMaskR)')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    ap.add_argument('--seed', type=int, default=15615)
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    ball = addr(a.tool, 'F-15615')
    alloc = addr(a.tool, 'F-89028')
    rnd = random.Random(a.seed)
    cases = []
    while len(cases) < a.cases:
        kind = rnd.choice([0, 1, 3, 3, 3])
        sc = rnd.choice([0.3, 1.0, 4.0])
        cen = [f32(rnd.uniform(-1, 1) * sc) for _ in range(3)]
        rad = f32(rnd.uniform(0.1, 1.5) * sc)
        crad = f32(rnd.uniform(0.05, 1.0) * sc)
        ln = f32(rnd.choice([0.0, rnd.uniform(0.1, 3.0) * sc]))
        size = [f32(rnd.uniform(0.05, 3.0) * sc) for _ in range(3)]
        R = f32(rnd.choice([0.2, 0.5, 1.0, rnd.uniform(0.02, 3.0)]))
        how = rnd.random()
        if kind == 3:
            h = [s / 2 for s in size]
            if how < 0.15:      # inside, nearer one face
                d = [rnd.uniform(-1, 1) * h[j] for j in range(3)]
            elif how < 0.4:     # over a face
                ax = rnd.randrange(3)
                d = [rnd.uniform(-1, 1) * h[j] for j in range(3)]
                d[ax] = rnd.choice([1, -1]) * (h[ax] + rnd.uniform(-0.2, 1.3) * R)
            elif how < 0.65:    # by an edge
                d = [rnd.choice([1, -1]) * (h[j] + rnd.uniform(-0.1, 1.0) * R) for j in range(3)]
                ax = rnd.randrange(3)
                d[ax] = rnd.uniform(-1, 1) * h[ax]
            elif how < 0.85:    # by a corner
                d = [rnd.choice([1, -1]) * (h[j] + rnd.uniform(-0.1, 1.0) * R) for j in range(3)]
            elif how < 0.93:    # exactly on an edge's line or a corner
                d = [rnd.choice([1, -1]) * h[j] for j in range(3)]
                if rnd.random() < 0.5:
                    ax = rnd.randrange(3)
                    d[ax] = rnd.uniform(-1, 1) * h[ax]
            else:               # anywhere
                d = [rnd.uniform(-3, 3) * sc for _ in range(3)]
        elif kind == 1:
            hl = ln / 2
            y = rnd.uniform(-hl - crad - R, hl + crad + R) * rnd.choice([1.0, 1.3])
            e = [rnd.gauss(0, 1), 0, rnd.gauss(0, 1)]
            en = math.hypot(e[0], e[2]) or 1.0
            dist = (crad + R) * rnd.choice([rnd.uniform(0.95, 1.05), rnd.uniform(0, 1), rnd.uniform(1, 2), 0.0])
            d = [e[0] / en * dist, y, e[2] / en * dist]
        else:
            e = [rnd.gauss(0, 1) for _ in range(3)]
            en = math.sqrt(sum(x * x for x in e)) or 1.0
            dist = (rad + R) * rnd.choice([rnd.uniform(0.95, 1.05), rnd.uniform(0, 1), rnd.uniform(1, 2), 0.0])
            d = [e[j] / en * dist for j in range(3)]
        c = [f32(cen[j] + d[j]) for j in range(3)]
        mask = rnd.choice([0] * 8 + [0x100, 0x200, 0x400])
        give = rnd.choice([1] * 9 + [0])
        row = [kind] + [fb(x) for x in cen] + [fb(rad), fb(crad), fb(ln)] + [fb(x) for x in size] + [fb(x) for x in c] + [fb(R), mask, give]
        assert len(row) == N_IN
        cases.append(row)
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%08x' % x for x in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    main_src = MAIN.replace('NIN_', str(N_IN)).replace('FAKE_MAT_', '0x%08Xu' % FAKE_MAT).replace('BALL_', ball)
    give = ('{uint32_t b; float x=float(ctx.f1.f64); memcpy(&b,&x,4); store(0x40802300u,b); x=float(ctx.f2.f64); memcpy(&b,&x,4);'
            'store(0x40802308u,b); store(0x40802304u,1); for(int k=0;k<3;k++) store(0x40802310u+4*k,load(ctx.r5.u32+4*k));'
            'bool g=GIVE_&&ctx.f1.f64>0.0; store(0x40802320u,g); ctx.r3.u64=g?0x40802400u:0;}')
    stubs = {'sub_' + alloc: give, **{'sub_' + addr(a.tool, f): 'throw std::runtime_error("curved geometry");' for f in CURVED}}
    exe = rd.build(idx, ['sub_' + ball], main_src, work, 'ball', image=img, stubs=stubs,
                   fakes={FAKE_MAT: 'ctx.r3.u64=(ctx.r4.u64+1)<<8;'})
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-500:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = [' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    nl = chr(10)
    (a.out_dir / 'ball_primitive_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split() for o in out]
    print(len(cases), 'cases;', sum(x[1] == '1' for x in res), 'alloc calls;', sum(x[0] == '1' for x in res), 'results 1')


if __name__ == '__main__':
    main()
