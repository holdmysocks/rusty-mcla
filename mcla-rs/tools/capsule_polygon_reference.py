"""F-43031, the undirected branch (query byte +212 = 0): a capsule (the query's +176 .. +192, radius +208) against one
polygon of a bound, run offline on the original's code with every callee whole - the segment-triangle test F-70182 ->
F-44891 -> F-33142, the ball-polygon test F-17804 at both ends, the edge test F-65002 (F-96822 with F-84741 / F-68765,
F-45690 with F-89261 / F-43156), the edge owner F-38934 and the bound's vertex fetch F-54719; only the result list's
alloc F-89028 is stubbed (it logs the depth f1, the cap f2 and the point r5 points at, and answers a record when the
case says so and the depth is above 0, as the real one does) and the bound's material call faked (byte << 8).

The start-up vector constants the image holds as zero are filled in as the running game has them (docs/reference/ai/
raceline.md): the xyz mask (all ones x3, 0), (0, 0, 0, 1) and +inf x4; the zero vectors have no initialiser. The lane
word F-45690 indexes with has no writer in the executable (one reader only) and stays 0. Built with -msse4.1 as the
recomp itself (vmsum3fp is the CPU's dpps).

A case: six vertices (the polygon's three, then one apex per neighbour), the capsule's ends and radius, the polygon's
area word, the three neighbour indices (1 .. 3, 0xFFFF none, 7 past the bound's count 4), the neighbours' area words,
the query's polygon index +60 (0, or 5: past the count), the material mask +88 and byte, the caller's f30 (two words:
F-44891 saves it as a double in the stack word that F-89261 reads as its stale line parameter when the capsule has no
length), and whether alloc gives a record.

Out per case: the result r3, alloc called, f1, f2, the point, the record (+0, +16 .. +56, +64, +68, +72, +76, +78, +80,
+82) and the stale stack word after. Writes <out-dir>/capsule_polygon_ref.txt. Test
`camprobe::tests::capsule_polygon_matches_the_original`."""
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
    # (the vertex fetch's unaligned load needs the right-hand mask table of the recomp's intrinsics)
    header = Path(SDK_INTRINSICS).read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    # the condition register's vector compare (the SDK's Condition::setFromMask) and its summary-overflow bit
    s = _vs()
    old = 'struct Condition {' + chr(10) + '    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    s = s.replace(old, '#include <simde/x86/sse4.1.h>' + chr(10) + 'struct Condition {' + chr(10) + '    bool so{}, eq{}, lt{}, gt{};' + chr(10) +
                  '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)
    return s + mask.replace('inline uint8_t', 'alignas(16) static uint8_t') + chr(10)


def _gpp(cmd, *a, **k):
    # the recomp's own x86 flag: vmsum3fp is dpps, (lanes 0 + 1) + (2 + 3)
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


rd.vector_shim = _shim
subprocess.run = _gpp
SDK_INTRINSICS = 'D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h'
FAKE_MAT = 0x7F000010
DIRECTED = ['F-25859', 'F-58482']   # the directed branch's callees (never reached with byte +212 = 0)
N_IN = 6 * 3 + 3 + 3 + 1 + 1 + 3 + 3 + 1 + 1 + 1 + 2 + 1
STALE = 0x2F00 - 944 - 192   # F-43031's frame (r1 0x2F00 less 944) less 192: F-65002's +128, F-17804's -192
MAIN = r'''
static const uint32_t Q=0x40800000, BND=0x40801000, VT=0x40801400, POLY=0x40801800, VTX=0x40801C00, MAT=0x40802000, REC=0x40802400;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}

int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 for(int k=0;k<4;k++) put(0x828609E0+4*k,0x7F800000);
 FILE*f=fopen(argv[2],"r"); unsigned w[NIN_];
 while(1){ int n=0; for(int i=0;i<NIN_;i++) n+=fscanf(f,"%x",&w[i]); if(n!=NIN_)break;
  rt_reset(); for(uint32_t a=Q;a<REC+0x100;a+=4) store(a,0);
  for(int k=0;k<6;k++){ for(int j=0;j<3;j++) store(VTX+16*k+4*j,w[3*k+j]); store(VTX+16*k+12,0); }
  for(int j=0;j<3;j++){ store(Q+176+4*j,w[18+j]); store(Q+192+4*j,w[21+j]); }
  store(Q+208,w[24]); *pointer(Q+212)=0;
  // polygon 0 and the three neighbours (k: vertices b, a of edge k-1 reversed, apex 2+k)
  store(POLY,w[25]); store16(POLY+4,0); store16(POLY+6,1); store16(POLY+8,2);
  for(int k=0;k<3;k++) store16(POLY+10+2*k,w[26+k]);
  for(int k=1;k<=3;k++){ uint32_t p=POLY+16*k; store(p,w[28+k]); store16(p+4,(k)%3); store16(p+6,k-1); store16(p+8,2+k);
    store16(p+10,0xFFFF); store16(p+12,0xFFFF); store16(p+14,0xFFFF); }
  store16(Q+60,w[32]); store(Q+88,w[33]);
  store(BND,VT); store(VT+48,FAKE_MAT_); store(BND+128,VTX); store(BND+140,POLY); store(BND+204,4); store(BND+216,MAT);
  for(int k=0;k<8;k++) *pointer(MAT+k)=uint8_t(w[34]);
  GIVE_=int(w[37]);
  for(uint32_t a=REC;a<REC+96;a+=4) store(a,0xDEADBEEF);
  store(0x40802300u,0); store(0x40802304u,0);
  Context c=fresh(); c.r3.u64=Q; c.r4.u64=POLY; c.r5.u64=VTX; c.r6.u64=0x1234; c.r7.u64=BND;
  { uint64_t b=(uint64_t(w[35])<<32)|w[36]; memcpy(&c.f30.f64,&b,8); }
  try{ sub_CAPS_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  int called=load(0x40802304u); bool rec=called&&GIVE_&&load(0x40802320u);
  printf("%d %d %08x %08x %08x %08x %08x",c.r3.s32,called,load(0x40802300u),load(0x40802308u),load(0x40802310u),load(0x40802314u),load(0x40802318u));
  for(int o: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",rec?load(REC+o):0u);
  printf(" %04x %04x %02x %04x", rec?load16(REC+76):0u, rec?load16(REC+78):0u, rec?*pointer(REC+80):0u, rec?load16(REC+82):0u);
  printf(" %08x\n",load(STALE_)); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--sdk', default=SDK_INTRINSICS, help='the recomp SDK intrinsics.h (VectorMaskR)')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    ap.add_argument('--seed', type=int, default=43031)
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    caps = addr(a.tool, 'F-43031')
    alloc = addr(a.tool, 'F-89028')
    rnd = random.Random(a.seed)
    sub = lambda p, q: [p[i] - q[i] for i in range(3)]
    cross = lambda p, q: [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]]
    norm = lambda p: math.sqrt(sum(x * x for x in p))
    unit = lambda p: [x / (norm(p) or 1.0) for x in p]
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
        apex = []
        nb_area = []
        for i in range(3):
            pa, pb = v[i], v[(i + 1) % 3]
            mid = [(pa[j] + pb[j]) / 2 for j in range(3)]
            out = unit(cross(sub(pb, pa), n))
            ang = rnd.uniform(-2.8, 2.8)
            d = rnd.uniform(0.2, 1.5) * sc
            x = [f32(mid[j] + d * (math.cos(ang) * out[j] + math.sin(ang) * n[j])) for j in range(3)]
            apex.append(x)
            nb_area.append(norm(cross(sub(x, pa), sub(pb, pa))) / 2)
        r = f32(rnd.choice([0.2, 0.3, 0.5, 1.0, rnd.uniform(0.05, 3.0)]) * (sc if rnd.random() < 0.3 else 1.0))
        # a point on or about the polygon: over the face, by an edge, by a corner, anywhere
        bc = [rnd.random() for _ in range(3)]
        s = sum(bc)
        on = [sum(bc[k] / s * v[k][j] for k in range(3)) for j in range(3)]
        kind = rnd.random()
        if kind < 0.35:
            i = rnd.randrange(3)
            t = rnd.uniform(-0.2, 1.2)
            pa, pb = v[i], v[(i + 1) % 3]
            out = unit(cross(sub(pb, pa), n))
            on = [pa[j] + t * (pb[j] - pa[j]) + out[j] * rnd.uniform(-r, r * 1.5) for j in range(3)]
        elif kind < 0.55:
            p = v[rnd.randrange(3)]
            on = [p[j] + rnd.uniform(-r, r) * 1.2 for j in range(3)]
        elif kind < 0.65:
            on = [on[j] + rnd.uniform(-2, 2) * sc for j in range(3)]
        # the capsule through or about that point: across the plane (steep or slanting), along it at a height, a short
        # one by it, or without length
        how = rnd.random()
        dirv = unit([rnd.gauss(0, 1) for _ in range(3)])
        if how < 0.35:
            tilt = rnd.uniform(0.15, 1.0)
            dirv = unit([dirv[j] * (1 - tilt) + n[j] * tilt * rnd.choice([1, -1]) * 2 for j in range(3)])
            l0, l1 = rnd.uniform(0.0, 2.0) * max(r, 0.3) * rnd.choice([1, 3]), rnd.uniform(0.0, 2.0) * max(r, 0.3) * rnd.choice([1, 3])
            h = rnd.choice([0.0, rnd.uniform(-1.2, 1.2) * r])
            pa = [on[j] + n[j] * h - dirv[j] * l0 for j in range(3)]
            pb = [on[j] + n[j] * h + dirv[j] * l1 for j in range(3)]
        elif how < 0.7:
            dirv = unit(cross(n, dirv))
            h = rnd.choice([rnd.uniform(-1.2, 1.2) * r, rnd.uniform(0, 1.05) * r, 0.0, rnd.uniform(-0.02, 0.02)])
            l0, l1 = rnd.uniform(0, 1.5) * sc, rnd.uniform(0, 1.5) * sc
            pa = [on[j] + n[j] * h - dirv[j] * l0 for j in range(3)]
            pb = [on[j] + n[j] * h + dirv[j] * l1 for j in range(3)]
        elif how < 0.93:
            h = rnd.uniform(-1.3, 1.3) * r
            l = rnd.uniform(0.001, 1.0) * r
            pa = [on[j] + n[j] * h for j in range(3)]
            pb = [pa[j] + dirv[j] * l for j in range(3)]
        else:
            h = rnd.uniform(-1.2, 1.2) * r
            pa = [on[j] + n[j] * h for j in range(3)]
            pb = list(pa) if rnd.random() < 0.6 else [pa[j] + dirv[j] * rnd.uniform(0, 1e-6) for j in range(3)]
        A = [f32(x) for x in pa]
        B = [f32(x) for x in pb]
        if rnd.random() < 0.5:
            A, B = B, A
        nbi = [rnd.choice([1 + i, 1 + i, 1 + i, 0xFFFF, 7]) for i in range(3)]
        q60 = rnd.choice([0] * 15 + [5])
        mask = rnd.choice([0] * 6 + [0x100, 0x200])
        matb = rnd.choice([0, 0, 1, 2, 3])
        f30 = struct.unpack('>Q', struct.pack('>d', rnd.choice([0.0, 0.5, 2.0, rnd.uniform(-1, 1), rnd.uniform(0, 1)])))[0]
        give = rnd.choice([1] * 9 + [0])
        row = ([fb(x) for p in v + apex for x in p] + [fb(x) for x in A + B] + [fb(r), fb(f32(area))] + nbi +
               [fb(f32(x)) for x in nb_area] + [q60, mask, matb, f30 >> 32, f30 & 0xFFFFFFFF, give])
        assert len(row) == N_IN
        cases.append(row)
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%08x' % x for x in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    main_src = (MAIN.replace('NIN_', str(N_IN)).replace('FAKE_MAT_', '0x%08Xu' % FAKE_MAT).replace('CAPS_', caps)
                .replace('STALE_', '0x%Xu' % STALE))
    give = ('{uint32_t b; float x=float(ctx.f1.f64); memcpy(&b,&x,4); store(0x40802300u,b); x=float(ctx.f2.f64); memcpy(&b,&x,4);'
            'store(0x40802308u,b); store(0x40802304u,1); for(int k=0;k<3;k++) store(0x40802310u+4*k,load(ctx.r5.u32+4*k));'
            'bool g=GIVE_&&ctx.f1.f64>0.0; store(0x40802320u,g); ctx.r3.u64=g?0x40802400u:0;}')
    exe = rd.build(idx, ['sub_' + caps], main_src, work, 'caps', image=img, stubs={'sub_' + alloc: give, **{'sub_' + addr(a.tool, f): 'throw std::runtime_error("directed branch");' for f in DIRECTED}},
                   fakes={FAKE_MAT: 'ctx.r3.u64=ctx.r4.u64<<8;'})
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-500:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = [' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    nl = chr(10)
    (a.out_dir / 'capsule_polygon_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split() for o in out]
    print(len(cases), 'cases;', sum(x[1] == '1' for x in res), 'alloc calls;', sum(x[0] == '1' for x in res), 'results 1')


if __name__ == '__main__':
    main()
