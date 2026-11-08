"""F-82650: a ball against one polygon of a bound (the sphere query's narrow phase), run offline on the original's code
with the edge owner F-38934 and the bound's vertex fetch run whole; only the result list's alloc (F-89028) is stubbed:
it logs the depth f1, the cap f2 and the point r5 points at, and answers the record (or none, by the case).

A case: six vertices (the polygon's three, then one apex per neighbour), the ball's centre (the query's +160) and
radius (+176), the polygon's area word, the three neighbour indices of its record (+10 / +12 / +14: 1 .. 3 the
neighbours folded about its edges, 0xFFFF none, 7 past the bound's count 4), the neighbours' area words, the query's
polygon index +60 (0, or 5: past the count, so the bound's own normal is (0, 0, 1)), the material mask +88 and the
polygon's material byte (the bound's material call answers byte << 8), the three stale words of the edge flags on the
stack (sp+160; an edge rejected by its owner leaves the later ones unwritten), and whether alloc gives a record.

Out per case: the result r3, alloc called, f1, f2, the point, the record (+0, +16 .. +56, +64, +68, +72, +76, +78,
+80, +82) and the three flag words after. Writes <out-dir>/ball_polygon_ref.txt. Test
`camprobe::tests::ball_polygon_matches_the_original`."""
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


def _shim():
    # (the vertex fetch's unaligned load needs the right-hand mask table of the recomp's intrinsics)
    # (the shared shim defines it itself now; add it only where it does not)
    s = _vs()
    if 'VectorMaskR[]' in s:
        return s
    header = Path(SDK_INTRINSICS).read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    return s + mask.replace('inline uint8_t', 'alignas(16) static uint8_t') + chr(10)


rd.vector_shim = _shim
SDK_INTRINSICS = 'D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h'
FAKE_MAT = 0x7F000010
N_IN = 6 * 3 + 3 + 1 + 1 + 3 + 3 + 1 + 1 + 1 + 3 + 1
MAIN = r'''
static const uint32_t Q=0x40800000, BND=0x40801000, VT=0x40801400, POLY=0x40801800, VTX=0x40801C00, MAT=0x40802000, REC=0x40802400;

int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 FILE*f=fopen(argv[2],"r"); unsigned w[NIN_];
 while(1){ int n=0; for(int i=0;i<NIN_;i++) n+=fscanf(f,"%x",&w[i]); if(n!=NIN_)break;
  rt_reset(); for(uint32_t a=Q;a<REC+0x100;a+=4) store(a,0);
  for(int k=0;k<6;k++){ for(int j=0;j<3;j++) store(VTX+16*k+4*j,w[3*k+j]); store(VTX+16*k+12,0); }
  for(int j=0;j<3;j++) store(Q+160+4*j,w[18+j]);
  store(Q+176,w[21]);
  // polygon 0 and the three neighbours (k: vertices b, a of edge k-1 reversed, apex 2+k)
  store(POLY,w[22]); store16(POLY+4,0); store16(POLY+6,1); store16(POLY+8,2);
  for(int k=0;k<3;k++) store16(POLY+10+2*k,w[23+k]);
  for(int k=1;k<=3;k++){ uint32_t p=POLY+16*k; store(p,w[25+k]); store16(p+4,(k)%3); store16(p+6,k-1); store16(p+8,2+k);
    store16(p+10,0xFFFF); store16(p+12,0xFFFF); store16(p+14,0xFFFF); }
  store16(Q+60,w[29]); store(Q+88,w[30]);
  store(BND,VT); store(VT+48,FAKE_MAT_); store(BND+128,VTX); store(BND+140,POLY); store(BND+204,4); store(BND+216,MAT);
  for(int k=0;k<8;k++) *pointer(MAT+k)=uint8_t(w[31]);
  for(int k=0;k<3;k++) store(0x2E10+4*k,w[32+k]);
  GIVE_=int(w[35]);
  for(uint32_t a=REC;a<REC+96;a+=4) store(a,0xDEADBEEF);
  store(0x40802300u,0); store(0x40802304u,0);
  Context c=fresh(); c.r3.u64=Q; c.r4.u64=POLY; c.r5.u64=VTX; c.r6.u64=0x1234; c.r7.u64=BND;
  try{ sub_BALL_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  int called=load(0x40802304u);
  printf("%d %d %08x %08x %08x %08x %08x",c.r3.s32,called,load(0x40802300u),load(0x40802308u),load(0x40802310u),load(0x40802314u),load(0x40802318u));
  for(int o: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",called&&GIVE_?load(REC+o):0u);
  printf(" %04x %04x %02x %04x", called&&GIVE_?load16(REC+76):0u, called&&GIVE_?load16(REC+78):0u, called&&GIVE_?*pointer(REC+80):0u, called&&GIVE_?load16(REC+82):0u);
  for(int k=0;k<3;k++) printf(" %08x",load(0x2E10+4*k));
  printf("\n"); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--sdk', default=SDK_INTRINSICS, help='the recomp SDK intrinsics.h (VectorMaskR)')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    ap.add_argument('--seed', type=int, default=82650)
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    ball = addr(a.tool, 'F-82650')
    alloc = addr(a.tool, 'F-89028')
    rnd = random.Random(a.seed)
    sub = lambda p, q: [p[i] - q[i] for i in range(3)]
    cross = lambda p, q: [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]]
    norm = lambda p: math.sqrt(sum(x * x for x in p))
    cases = []
    while len(cases) < a.cases:
        sc = rnd.choice([0.5, 2.0, 8.0, 40.0])
        v = [[f32(rnd.uniform(-sc, sc)) for _ in range(3)] for _ in range(3)]
        if rnd.random() < 0.3:
            y = f32(rnd.uniform(-2, 2))
            v = [[p[0], y, p[2]] for p in v]
        if rnd.random() < 0.15:
            x = f32(rnd.uniform(-2, 2))
            v = [[x, p[1], p[2]] for p in v]
        cr = cross(sub(v[2], v[1]), sub(v[0], v[1]))
        area = norm(cr) / 2
        if area < 1e-4:
            continue
        n = [c / (2 * area) for c in cr]
        # neighbour apexes: across each edge, folded by a random angle
        apex = []
        nb_area = []
        for i in range(3):
            pa, pb = v[i], v[(i + 1) % 3]
            mid = [(pa[j] + pb[j]) / 2 for j in range(3)]
            e = sub(pb, pa)
            out = cross(e, n)
            lo = norm(out) or 1.0
            out = [x / lo for x in out]
            ang = rnd.uniform(-2.8, 2.8)
            d = rnd.uniform(0.2, 1.5) * sc
            x = [f32(mid[j] + d * (math.cos(ang) * out[j] + math.sin(ang) * n[j])) for j in range(3)]
            apex.append(x)
            # the neighbour's record: (b, a, apex)
            c2 = cross(sub(x, pa), sub(pb, pa))
            nb_area.append(norm(c2) / 2)
        r = f32(rnd.choice([0.2, 0.3, 0.5, 1.0, rnd.uniform(0.05, 3.0)]) * (sc if rnd.random() < 0.3 else 1.0))
        # the centre: over the face, near an edge or a corner, anywhere; at a height about the plane
        kind = rnd.random()
        bc = [rnd.random() for _ in range(3)]
        s = sum(bc)
        bc = [x / s for x in bc]
        on = [sum(bc[k] * v[k][j] for k in range(3)) for j in range(3)]
        if kind < 0.35:
            i = rnd.randrange(3)
            t = rnd.uniform(-0.2, 1.2)
            pa, pb = v[i], v[(i + 1) % 3]
            on = [pa[j] + t * (pb[j] - pa[j]) for j in range(3)]
            e = sub(pb, pa)
            out = cross(e, n)
            lo = norm(out) or 1.0
            on = [on[j] + out[j] / lo * rnd.uniform(-r, r * 1.3) for j in range(3)]
        elif kind < 0.55:
            p = v[rnd.randrange(3)]
            on = [p[j] + rnd.uniform(-r, r) for j in range(3)]
        elif kind < 0.65:
            on = [on[j] + rnd.uniform(-2, 2) * sc for j in range(3)]
        h = rnd.choice([rnd.uniform(-1.2, 1.2) * r, rnd.uniform(0, 1.05) * r, 0.0, rnd.uniform(-0.02, 0.02)])
        c = [f32(on[j] + n[j] * h) for j in range(3)]
        if rnd.random() < 0.02:
            c = [f32(x) for x in rnd.choice(v)]
        nbi = [rnd.choice([1 + i, 1 + i, 1 + i, 0xFFFF, 7]) for i in range(3)]
        q60 = rnd.choice([0] * 15 + [5])
        mask = rnd.choice([0] * 6 + [0x100, 0x200])
        matb = rnd.choice([0, 0, 1, 2, 3])
        stale = [rnd.choice([0, 1, 0xFFFFFFFF, 0xFFFFFFFE, rnd.getrandbits(32)]) for _ in range(3)]
        give = rnd.choice([1] * 9 + [0])
        row = ([fb(x) for p in v + apex for x in p] + [fb(x) for x in c] + [fb(r), fb(f32(area))] + nbi +
               [fb(f32(x)) for x in nb_area] + [q60, mask, matb] + stale + [give])
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
            'ctx.r3.u64=GIVE_?0x40802400u:0;}')
    exe = rd.build(idx, ['sub_' + ball], main_src, work, 'ball', image=img, stubs={'sub_' + alloc: give},
                   fakes={FAKE_MAT: 'ctx.r3.u64=ctx.r4.u64<<8;'})
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-500:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = [' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    nl = chr(10)
    (a.out_dir / 'ball_polygon_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split() for o in out]
    print(len(cases), 'cases;', sum(x[1] == '1' for x in res), 'alloc calls;', sum(x[0] == '1' for x in res), 'results 1')


if __name__ == '__main__':
    main()
