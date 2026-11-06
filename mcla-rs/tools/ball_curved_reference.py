"""F-15615 case 5: a ball (the query's centre +160 in the bound's frame, radius +176) against a curved bound (type 5) - a
car's wheel - run offline on the original's code whole: the ball made a sphere bound (F-80030) at the query's centre,
the penetration routine F-96503 (the 42 ways out, the detector's set-up F-15933 and loop F-33272 with the curved bound's
support, the sphere's, the simplex routines and the result object's call), the sphere's destructor (F-48308), the
depth / normal rules after it and the result list. Only the list's alloc F-89028 is stubbed (it logs the depth f1, the
cap f2 and the point r5 points at, and answers a record when the case says so and the depth is above 0, as the real one
does) and the bound's material call faked ((index + 1) << 8). The vector constants the image holds as zero are made by
the original's own static initialisers (the ten of gjk_reference.py).

The wheel: the unit wheel bound of a car file (`0x8a43126c.r83`, the same in every car: its header, four corners, six
curved edges and four curved faces), resized as the game resizes it (F-55382's factors, the corners by plain products,
F-17852 on each edge and F-22892 with F-95825 on each face) first to the size every wheel bound is made with (TireRadius
0.3, TireWidth 0.1) and then to a tyre a set-up gives (`simtune::wheel_radius`: rim diameter x 0.5 + profile x width -
0.03175, width 0.195 + 0.01 k; rims 12 - 28, profiles 0 - 13, widths 0 - 16), the port's `gjk::wheel_shape`.

A case: the tyre's radius and width, the ball's centre and radius, the material mask +88, whether alloc gives a record.
Centres by the tread, the rims, the sides, inside the core, on the axle, far.

Out per case: the result r3, alloc called, f1, f2, the point, the record (+0, +16 .. +24, +32 .. +40, +48 .. +56, +64,
+68, +72, +76, +78, +80, +82). Writes <out-dir>/ball_curved_ref.txt. Test `camprobe::tests::ball_curved_matches_the_original`."""
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


def ffma(a, b, c):
    # one rounding of a b + c (exact in double for these sizes), then to single
    return f32(math.fma(a, b, c)) if hasattr(math, 'fma') else f32(a * b + c)


import recomp_closure
recomp_closure.EXTRA += 'static int GIVE_;' + chr(10)
recomp_closure.EXTRA += (
    'namespace rex { namespace ppc {' + chr(10) +
    'inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {' + chr(10) +  # the SDK's intrinsics.h
    '  simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));' + chr(10) +
    '  simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8);' + chr(10) +
    '  xmm0 = simde_mm_srli_epi32(xmm0, 31);' + chr(10) +
    '  xmm0 = simde_mm_add_epi32(xmm0, xmm1);' + chr(10) +
    '  xmm0 = simde_mm_srai_epi32(xmm0, 8);' + chr(10) +
    '  xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000));' + chr(10) +
    '  simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);' + chr(10) +
    '  return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1));' + chr(10) +
    '} } }' + chr(10))
_vs = rd.vector_shim
_run = subprocess.run


def _shim():
    # (unaligned splats: the right-hand mask table of the recomp's intrinsics; the condition register's vector compare and
    # its summary-overflow bit; vslw128 as simde's portable AVX2 shift)
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
    # the recomp's own x86 flag: vmsum3fp is dpps, (lanes 0 + 1) + (2 + 3)
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


rd.vector_shim = _shim
subprocess.run = _gpp
SDK_INTRINSICS = 'D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h'
FAKE_MAT = 0x7F000010
# the static initialisers of the vector constants the routines read (BSS in the image), as gjk_reference.py runs them
INITS = ['F-50173', 'F-33402', 'F-26199', 'F-55749', 'F-97349', 'F-70580', 'F-79721', 'F-64228', 'F-24192', 'F-92824']
# the resizing of the wheel bound (F-17852 edges, F-22892 faces with F-95825)
RESIZE_IDS = ['F-17852', 'F-22892']
N_IN = 2 + 3 + 1 + 1 + 1
MAIN = r'''
static const uint32_t Q=0x40800000, BND=0x40801000, VT=0x40801400, VERTS=0x40801600, EDGES=0x40801700, FACES=0x40801900, REC=0x40802400;
static void bytes(uint32_t a,const char*hex){ for(size_t i=0;hex[2*i];i++){ unsigned v; sscanf(hex+2*i,"%2x",&v); *pointer(a+i)=uint8_t(v);} }
static float gf(uint32_t a){ uint32_t b=load(a); float v; memcpy(&v,&b,4); return v; }
static void pf(uint32_t a,float v){ uint32_t b; memcpy(&b,&v,4); store(a,b); }
static void resize(float radius,float width){
  float margin=gf(BND+112);
  float vx=gf(VERTS), vy=gf(VERTS+4), vz=gf(VERTS+8);
  float want=radius-margin, half=width*gf(0x82003A24);
  float rv2=float(std::fma(double(vz),double(vz),double(float(vy*vy))));
  float rv=rv2>0.0f ? float(std::sqrt(double(rv2))) : 0.0f;
  if(!(std::fabs(float(rv-want))>gf(0x820022B4) || std::fabs(float(std::fabs(vx)-half))>gf(0x820022B4))) return;
  float sx=half/std::fabs(vx), sy=want/rv;
  for(int i=0;i<4;i++){ pf(VERTS+16*i,sx*gf(VERTS+16*i)); pf(VERTS+16*i+4,gf(VERTS+16*i+4)*sy); pf(VERTS+16*i+8,gf(VERTS+16*i+8)*sy); }
  for(int i=0;i<6;i++){ Context c=fresh(); c.r3.u64=EDGES+48*i; c.r7.u64=BND; c.f1.f64=sx; c.f2.f64=sy; c.f3.f64=sy; sub_EDGE_(c,nullptr); }
  for(int i=0;i<4;i++){ Context c=fresh(); c.r3.u64=FACES+96*i; c.r7.u64=BND; c.r8.u64=EDGES; c.f1.f64=sx; c.f2.f64=sy; c.f3.f64=sy; sub_FACE_(c,nullptr); }
}

int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 rt_reset();
 { INITCALLS }
 if(load(0x82860950)!=0xffffffffu || load(0x828609E0)!=0x7F800000u) { printf("EXC constants\n"); return 1; }
 FILE*f=fopen(argv[2],"r"); unsigned w[NIN_];
 while(1){ int n=0; for(int i=0;i<NIN_;i++) n+=fscanf(f,"%x",&w[i]); if(n!=NIN_)break;
  rt_reset(); for(uint32_t a=Q;a<REC+0x100;a+=4) store(a,0);
  bytes(BND,"UNITHEAD"); bytes(VERTS,"UNITVERTS"); bytes(EDGES,"UNITEDGES"); bytes(FACES,"UNITFACES");
  store(BND+128,VERTS); store(BND+132,0); store(BND+224,FACES); store(BND+228,EDGES);
  float r,wd; memcpy(&r,&w[0],4); memcpy(&wd,&w[1],4);
  resize(0.3f,0.1f); resize(r,wd);
  store(BND,VT); store(VT+48,FAKE_MAT_);
  for(int j=0;j<3;j++) store(Q+160+4*j,w[2+j]);
  store(Q+176,w[5]); store(Q+88,w[6]);
  GIVE_=int(w[7]);
  for(uint32_t a=REC;a<REC+96;a+=4) store(a,0xDEADBEEF);
  store(0x40802300u,0); store(0x40802304u,0);
  Context c=fresh(); c.r3.u64=Q; c.r4.u64=BND; c.r5.u64=0x1234;
  try{ sub_BALL_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  int called=load(0x40802304u); bool rec=called&&GIVE_&&load(0x40802320u);
  printf("%d %d %08x %08x %08x %08x %08x",c.r3.s32&0xFF,called,load(0x40802300u),load(0x40802308u),load(0x40802310u),load(0x40802314u),load(0x40802318u));
  for(int o: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",rec?load(REC+o):0u);
  printf(" %04x %04x %02x %04x\n", rec?load16(REC+76):0u, rec?load16(REC+78):0u, rec?*pointer(REC+80):0u, rec?load16(REC+82):0u); } }
'''


def unit_wheel(data: Path):
    """The unit wheel bound of a car file: header, corners, curved edges and faces (as gjk_reference.py reads it)."""
    raw = (data / 'resources/vehicle/vp_sln_s281_06/0x8a43126c.r83').read_bytes()
    assert raw[:4] == b'MCLR'
    size = struct.unpack_from('<I', raw, 12)[0]
    d = raw[16:16 + size]
    o = 0x106560
    u32 = lambda a: struct.unpack_from('>I', d, a)[0]
    assert d[o + 4] == 5 and u32(o + 0xC8) == 4 and u32(o + 0xE8) == 4 and u32(o + 0xEC) == 6, 'not the curved wheel bound'
    at = lambda a: u32(a) & 0xfffffff
    v, f, e = at(o + 0x80), at(o + 0xE0), at(o + 0xE4)
    return d[o:o + 0xF0], d[v:v + 64], d[e:e + 288], d[f:f + 384]


RIM = [0x3ea90ffa, 0x3eb61134, 0x3ec3126f, 0x3ed013a9, 0x3edd14e4, 0x3eea161f, 0x3ef71759, 0x3f020c4a, 0x3f088ce7, 0x3f0f0d84,
       0x3f158e21, 0x3f1c0ebe, 0x3f228f5c, 0x3f290ff9, 0x3f2f9096, 0x3f361134, 0x3f3c91d1]


def tyre(rnd):
    """A tyre a set-up gives (simtune::wheel_radius): (radius, width)."""
    w = f32(0.195 + 0.01 * rnd.randrange(17))
    p = f32(0.20 + 0.05 * rnd.randrange(14))
    t = f32(p * w - struct.unpack('>f', struct.pack('>I', 0x3d020c4a))[0])
    r = f32(struct.unpack('>f', struct.pack('>I', RIM[rnd.randrange(17)]))[0] * 0.5 + t)
    return r, w


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, default=Path('D:/MCLA/extracted/cache'), help='the extracted resources (a car .r83)')
    ap.add_argument('--sdk', default=SDK_INTRINSICS, help='the recomp SDK intrinsics.h (VectorMaskR)')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    ap.add_argument('--seed', type=int, default=15616)
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    ball = addr(a.tool, 'F-15615')
    alloc = addr(a.tool, 'F-89028')
    rnd = random.Random(a.seed)
    cases = []
    while len(cases) < a.cases:
        r, w = tyre(rnd)
        core = r - 0.04
        R = f32(rnd.choice([0.05, 0.2, 0.5, 1.0, rnd.uniform(0.02, 1.6)]))
        how = rnd.random()
        ang = rnd.uniform(0, 2 * math.pi)
        if how < 0.35:      # by the tread
            x = rnd.uniform(-0.6, 0.6) * w
            rho = r + R * rnd.choice([rnd.uniform(0.9, 1.1), rnd.uniform(-0.5, 1.0), rnd.uniform(1.0, 1.6)])
        elif how < 0.6:     # by a side
            x = rnd.choice([-1, 1]) * (w / 2 + R * rnd.choice([rnd.uniform(0.9, 1.1), rnd.uniform(-0.5, 1.0), rnd.uniform(1.0, 1.6)]))
            rho = rnd.uniform(0, r * 1.1)
        elif how < 0.8:     # by a rim
            x = rnd.choice([-1, 1]) * (w / 2 + R * rnd.uniform(0.3, 1.0) * 0.7)
            rho = core + R * rnd.uniform(0.3, 1.0) * 0.7
        elif how < 0.9:     # inside, or on the axle
            x = rnd.uniform(-0.5, 0.5) * w
            rho = rnd.choice([0.0, rnd.uniform(0, core)])
        else:               # anywhere
            x = rnd.uniform(-2, 2) * (w + R)
            rho = rnd.uniform(0, 2 * (r + R))
        c = [f32(x), f32(rho * math.sin(ang)), f32(rho * math.cos(ang))]
        if rnd.random() < 0.05:
            c[rnd.randrange(3)] = 0.0
        mask = rnd.choice([0] * 8 + [0x100, 0x200])
        give = rnd.choice([1] * 9 + [0])
        row = [fb(r), fb(w)] + [fb(x) for x in c] + [fb(R), mask, give]
        assert len(row) == N_IN
        cases.append(row)
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%08x' % x for x in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    inits = [addr(a.tool, f) for f in INITS]
    RESIZE = [addr(a.tool, f) for f in RESIZE_IDS]
    head, verts, edges, faces = unit_wheel(a.data)
    main_src = (MAIN.replace('NIN_', str(N_IN)).replace('FAKE_MAT_', '0x%08Xu' % FAKE_MAT).replace('BALL_', ball)
                .replace('EDGE_', RESIZE[0]).replace('FACE_', RESIZE[1])
                .replace('INITCALLS', ' '.join('{Context g=fresh(); sub_%s(g,nullptr);}' % x for x in inits))
                .replace('UNITHEAD', head.hex()).replace('UNITVERTS', verts.hex()).replace('UNITEDGES', edges.hex())
                .replace('UNITFACES', faces.hex()))
    give = ('{uint32_t b; float x=float(ctx.f1.f64); memcpy(&b,&x,4); store(0x40802300u,b); x=float(ctx.f2.f64); memcpy(&b,&x,4);'
            'store(0x40802308u,b); store(0x40802304u,1); for(int k=0;k<3;k++) store(0x40802310u+4*k,load(ctx.r5.u32+4*k));'
            'bool g=GIVE_&&ctx.f1.f64>0.0; store(0x40802320u,g); ctx.r3.u64=g?0x40802400u:0;}')
    stubs = {'sub_' + alloc: give}
    exe = rd.build(idx, ['sub_' + ball] + ['sub_' + x for x in inits + RESIZE], main_src, work, 'ballc', image=img, stubs=stubs,
                   vtables=[(0x82083460, 2)], fakes={FAKE_MAT: 'ctx.r3.u64=(ctx.r4.u64+1)<<8;'})
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stdout[-300:], r.stderr[-500:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = [' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    nl = chr(10)
    (a.out_dir / 'ball_curved_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split() for o in out]
    print(len(cases), 'cases;', sum(x[1] == '1' for x in res), 'alloc calls;', sum(x[0] == '1' for x in res), 'results 1')


if __name__ == '__main__':
    main()
