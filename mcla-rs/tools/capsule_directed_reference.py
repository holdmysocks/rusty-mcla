"""F-11472, the directed branch (the query's byte +212 = 1, as F-13494 sets it for the cameras' swept ball), on a
sphere (type 0: centroid +48, radius +8, through F-53035), a capsule (type 1: radius +128, length +144 along y, through
F-40312) or a box bound (type 3: size +208; the crossings, F-79106 directed with F-25936, then F-14203 / F-43026 for a
corner, F-62040 / F-58482 with the wedge test F-22406 for an edge, the face rule): the ball of radius +208 swept from
+176 to +192, run offline on the original's code whole. Only the result list's alloc F-89028 is stubbed (it logs f1,
f2 and the point r5 points at, and answers a record when the case says so and the depth is above 0), the bound's
material call faked ((index + 1) << 8), and the curved geometry's helpers (F-80030, F-60260, F-81943, F-48308) throw if
reached. The start-up constants the image holds as zero are filled in (the xyz mask, (0, 0, 0, 1), +inf x4). The
stack below the caller's frame is filled with the word --fill (NaN by default) before each case: the box's pick of a
crossing can read two words its frame leaves unset (docs/reference/cameras/probe_rule.md "F-11472 directed").

A case: the type, the bound's centroid, sphere radius, capsule radius and length, box size; the sweep's ends and radius;
the material mask +88; whether alloc gives a record. Sweeps towards, away from, along, by and through the bound, from
inside it, upright ones, ones along a box face or axis, and ones without length.

Out per case: the result r3, alloc called, f1, f2, the point, the record (+0, +16 .. +24, +32 .. +40, +48 .. +56, +64,
+68, +72, +76, +78, +80, +82). Writes <out-dir>/capsule_directed_ref.txt. Test
`camprobe::tests::capsule_directed_matches_the_original`."""
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
# (the box's helpers the closure pulls in, never run here, convert with the SDK's unsigned-to-float helper)
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
    # its summary-overflow bit for the box's helpers the closure pulls in; vslw128 as simde's portable AVX2 shift)
    header = Path(SDK_INTRINSICS).read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    s = '#include <simde/x86/avx2.h>' + chr(10) + _vs()
    old = 'struct Condition {' + chr(10) + '    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    s = s.replace(old, 'struct Condition {' + chr(10) + '    bool so{}, eq{}, lt{}, gt{};' + chr(10) +
                  '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)
    if 'VectorMaskR' in s:  # (the shared shim holds it since)
        return s
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
NOT_RUN = ['F-80030', 'F-60260', 'F-81943', 'F-48308']
N_IN = 1 + 3 + 3 + 3 + 3 + 3 + 1 + 1 + 1
MAIN = r'''
static const uint32_t Q=0x40800000, BND=0x40801000, VT=0x40801400, REC=0x40802400;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}

int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 for(int k=0;k<4;k++) put(0x828609E0+4*k,0x7F800000);
 FILE*f=fopen(argv[2],"r"); unsigned w[NIN_];
 while(1){ int n=0; for(int i=0;i<NIN_;i++) n+=fscanf(f,"%x",&w[i]); if(n!=NIN_)break;
  rt_reset(); for(uint32_t a=0x1800;a<0x2f00;a+=4) store(a,FILL_); for(uint32_t a=Q;a<REC+0x100;a+=4) store(a,0);
  store(BND,VT); store(VT+48,FAKE_MAT_); *pointer(BND+4)=uint8_t(w[0]);
  for(int j=0;j<3;j++) store(BND+48+4*j,w[1+j]);
  store(BND+8,w[4]); store(BND+128,w[5]); store(BND+144,w[6]);
  for(int j=0;j<3;j++) store(BND+208+4*j,w[7+j]);
  for(int j=0;j<3;j++){ store(Q+176+4*j,w[10+j]); store(Q+192+4*j,w[13+j]); }
  store(Q+208,w[16]); *pointer(Q+212)=1; store(Q+88,w[17]);
  GIVE_=int(w[18]);
  for(uint32_t a=REC;a<REC+96;a+=4) store(a,0xDEADBEEF);
  store(0x40802300u,0); store(0x40802304u,0);
  Context c=fresh(); c.r3.u64=Q; c.r4.u64=BND; c.r5.u64=0x1234;
  try{ sub_CAPS_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  int called=load(0x40802304u); bool rec=called&&GIVE_&&load(0x40802320u);
  printf("%d %d %08x %08x %08x %08x %08x",c.r3.s32&0xFF,called,load(0x40802300u),load(0x40802308u),load(0x40802310u),load(0x40802314u),load(0x40802318u));
  for(int o: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",rec?load(REC+o):0u);
  printf(" %04x %04x %02x %04x\n", rec?load16(REC+76):0u, rec?load16(REC+78):0u, rec?*pointer(REC+80):0u, rec?load16(REC+82):0u); } }
'''


def sphere_capsule(rnd, unit):
    kind = rnd.choice([0, 1, 1])
    sc = rnd.choice([0.3, 1.0, 4.0])
    cen = [f32(rnd.uniform(-1, 1) * sc) for _ in range(3)]
    rad = f32(rnd.uniform(0.1, 1.5) * sc)
    crad = f32(rnd.uniform(0.05, 1.0) * sc)
    ln = f32(rnd.choice([0.0, rnd.uniform(0.1, 3.0) * sc, rnd.uniform(0.1, 3.0) * sc]))
    R = f32(rnd.choice([0.2, 0.5, 1.0, rnd.uniform(0.02, 3.0)]))
    reach = (rad if kind == 0 else crad) + R
    # a point the sweep passes: about the bound's surface, inside it, on its centre or axis, far
    how = rnd.random()
    e = unit([rnd.gauss(0, 1) for _ in range(3)])
    y = rnd.uniform(-ln / 2 - reach, ln / 2 + reach) if kind == 1 else 0.0
    if kind == 1 and rnd.random() < 0.7:
        e = unit([e[0], 0.0, e[2]])
    if how < 0.1:
        dist = 0.0
    else:
        dist = reach * rnd.choice([rnd.uniform(0.95, 1.05), rnd.uniform(0, 1), rnd.uniform(1, 2.5)])
    on = [cen[0] + e[0] * dist, cen[1] + y + e[1] * dist, cen[2] + e[2] * dist]
    # the sweep: upright, across, straight at the bound, any way; through that point, ending at it, or no length
    dk = rnd.random()
    if dk < 0.2:
        dv = unit([rnd.uniform(-0.005, 0.005), 1.0, rnd.uniform(-0.005, 0.005)])
    elif dk < 0.4:
        dv = unit([rnd.gauss(0, 1), 0.0, rnd.gauss(0, 1)])
    elif dk < 0.55:
        dv = [-x for x in e]
    else:
        dv = unit([rnd.gauss(0, 1) for _ in range(3)])
    L = rnd.choice([rnd.uniform(0.05, 1.0), rnd.uniform(0.5, 5.0), rnd.uniform(2.0, 10.0)]) * sc if rnd.random() < 0.95 else 0.0
    u0 = rnd.choice([0.0, 1.0, 0.5, rnd.uniform(-0.3, 1.3), rnd.uniform(0.6, 1.0)])
    A = [f32(on[j] - dv[j] * L * u0) for j in range(3)]
    B = [f32(A[j] + dv[j] * L) for j in range(3)]
    if rnd.random() < 0.35:
        A, B = B, A
    return [kind] + [fb(x) for x in cen] + [fb(rad), fb(crad), fb(ln)] + [0, 0, 0] + [fb(x) for x in A + B] + [fb(R)]


def box(rnd, unit):
    sgn = lambda: rnd.choice([-1.0, 1.0])
    sc = rnd.choice([0.3, 1.0, 4.0])
    cen = [f32(rnd.uniform(-1, 1) * sc) for _ in range(3)]
    size = [f32(rnd.uniform(0.1, 3.0) * sc) for _ in range(3)]
    if rnd.random() < 0.1:
        size = [size[0]] * 3
    h = [x / 2 for x in size]
    R = f32(rnd.choice([0.2, 0.5, 1.0, rnd.uniform(0.02, 3.0)]))
    # a point the sweep passes: off a corner, an edge, a face (outwards), inside, anywhere
    how = rnd.random()
    off = rnd.choice([0.0, rnd.uniform(0, 1), rnd.uniform(0.9, 1.1), rnd.uniform(1, 2.5)]) * R
    e = unit([rnd.gauss(0, 1) for _ in range(3)])
    if how < 0.25:
        on = [sgn() * h[k] for k in range(3)]
        e = unit([abs(e[k]) * (1 if on[k] > 0 else -1) for k in range(3)])
    elif how < 0.55:
        k = rnd.randrange(3)
        on = [sgn() * h[j] for j in range(3)]
        on[k] = rnd.uniform(-h[k], h[k])
        e = unit([abs(e[j]) * (1 if on[j] > 0 else -1) if j != k else 0.0 for j in range(3)])
    elif how < 0.8:
        k = rnd.randrange(3)
        on = [rnd.uniform(-h[j], h[j]) for j in range(3)]
        on[k] = sgn() * h[k]
        if rnd.random() < 0.7:
            e = [0.0, 0.0, 0.0]
            e[k] = 1.0 if on[k] > 0 else -1.0
    elif how < 0.9:
        on = [rnd.uniform(-h[j], h[j]) * rnd.choice([1.0, 0.5, 0.1]) for j in range(3)]
        off = 0.0
    else:
        on = [rnd.uniform(-2 * h[j] - R, 2 * h[j] + R) for j in range(3)]
    if rnd.random() < 0.1:
        off = 0.0
    on = [on[j] + e[j] * off for j in range(3)]
    # the sweep: along an axis, in a face plane (zero parts), towards the feature, any way, or no length
    dk = rnd.random()
    if dk < 0.12:
        dv = [0.0, 0.0, 0.0]
        dv[rnd.randrange(3)] = sgn()
    elif dk < 0.3:
        dv = [rnd.gauss(0, 1) for _ in range(3)]
        dv[rnd.randrange(3)] = 0.0
        dv = unit(dv)
    elif dk < 0.55:
        dv = unit([-e[j] + rnd.gauss(0, 0.3) for j in range(3)])
    else:
        dv = unit([rnd.gauss(0, 1) for _ in range(3)])
    L = rnd.choice([rnd.uniform(0.05, 1.0), rnd.uniform(0.5, 3.0), rnd.uniform(2.0, 8.0)]) * sc if rnd.random() < 0.95 else 0.0
    u0 = rnd.choice([0.0, 1.0, 0.5, rnd.uniform(-0.3, 1.3), rnd.uniform(0.6, 1.0)])
    A = [f32(cen[j] + on[j] - dv[j] * L * u0) for j in range(3)]
    B = [f32(A[j] + dv[j] * L) for j in range(3)]
    if rnd.random() < 0.3:
        A, B = B, A
    return [3] + [fb(x) for x in cen] + [0, 0, 0] + [fb(x) for x in size] + [fb(x) for x in A + B] + [fb(R)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--sdk', default=SDK_INTRINSICS, help='the recomp SDK intrinsics.h (VectorMaskR)')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=9000)
    ap.add_argument('--seed', type=int, default=13494)
    ap.add_argument('--fill', default='7FC00000', help='the word the stack is filled with before each case (hex)')
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    fill = int(a.fill, 16)
    caps = addr(a.tool, 'F-11472')
    alloc = addr(a.tool, 'F-89028')
    rnd = random.Random(a.seed)
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    cases = []
    while len(cases) < a.cases:
        row = sphere_capsule(rnd, unit) if len(cases) % 5 < 2 else box(rnd, unit)
        row += [rnd.choice([0] * 8 + [0x100, 0x200]), rnd.choice([1] * 9 + [0])]
        assert len(row) == N_IN
        cases.append(row)
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%08x' % x for x in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    main_src = (MAIN.replace('NIN_', str(N_IN)).replace('FAKE_MAT_', '0x%08Xu' % FAKE_MAT).replace('CAPS_', caps)
                .replace('FILL_', '0x%08Xu' % fill))
    give = ('{uint32_t b; float x=float(ctx.f1.f64); memcpy(&b,&x,4); store(0x40802300u,b); x=float(ctx.f2.f64); memcpy(&b,&x,4);'
            'store(0x40802308u,b); store(0x40802304u,1); for(int k=0;k<3;k++) store(0x40802310u+4*k,load(ctx.r5.u32+4*k));'
            'bool g=GIVE_&&ctx.f1.f64>0.0; store(0x40802320u,g); ctx.r3.u64=g?0x40802400u:0;}')
    stubs = {'sub_' + alloc: give, **{'sub_' + addr(a.tool, f): 'throw std::runtime_error("not this branch");' for f in NOT_RUN}}
    exe = rd.build(idx, ['sub_' + caps], main_src, work, 'caps', image=img, stubs=stubs,
                   fakes={FAKE_MAT: 'ctx.r3.u64=(ctx.r4.u64+1)<<8;'})
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-500:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = [' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    nl = chr(10)
    (a.out_dir / 'capsule_directed_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split() for o in out]
    for t in (0, 1, 3):
        sel = [x for c, x in zip(cases, res) if c[0] == t]
        print('type', t, len(sel), 'cases;', sum(x[1] == '1' for x in sel), 'alloc calls;', sum(x[0] == '1' for x in sel), 'results 1')


if __name__ == '__main__':
    main()
