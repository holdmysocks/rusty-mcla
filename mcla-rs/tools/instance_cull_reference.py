"""The level query's cull of one level object (F-62425 with the culler's +100 = 0 -> F-92940, with F-43536 for the
capsule), after the culler is set up from the query as the drivers do it (F-36470 for the sphere query, F-21683 for the
swept capsule), run offline on the original's code whole - nothing stubbed.

A case: the query (a ball: centre +144, radius +176; or a capsule: ends +144 / +160, radius +208), the culler's masks
(+84 include, +88 excluded types, +92 type flags, +96 states, a word), the object's state bit (F-62425's r5), its archetype's
type and include words (+16 / +20), its bound's centre (+48) and radius (+8) and its instance matrix (object +16: three
rows and the place). Bounds about the query's surface, inside, far off; capsules without length; masks that pass and
that fail each way. The start-up constants the image holds as zero are filled in (the xyz mask, (0, 0, 0, 1), +inf x4).
Out per case: the culler's volume words (+0 .. +48, +80) and the result. Writes <out-dir>/instance_cull_ref.txt. Test
`camprobe::tests::instance_cull_matches_the_original`. With --points: balls whose radius is not above 0 only (F-36470's
point volume, type 5), written to <out-dir>/instance_cull_point_ref.txt (test `camprobe::tests::point_cull_matches_the_original`)."""
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


_run = subprocess.run


def _gpp(cmd, *a, **k):
    # the recomp's own x86 flag: vmsum3fp is dpps, (lanes 0 + 1) + (2 + 3)
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


subprocess.run = _gpp
_vs = rd.vector_shim


def _shim():
    # the condition register's vector compare (the SDK's Condition::setFromMask) and its summary-overflow bit; the
    # bound's radius is an unaligned load (the right-hand mask table of the recomp's intrinsics)
    header = Path(SDK_INTRINSICS).read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    # (vslw128 is the AVX2 per-lane shift in the recomp's intrinsics; simde's portable form)
    s = '#include <simde/x86/avx2.h>' + chr(10) + _vs()
    old = 'struct Condition {' + chr(10) + '    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    s = s.replace(old, '#include <simde/x86/sse4.1.h>' + chr(10) + 'struct Condition {' + chr(10) + '    bool so{}, eq{}, lt{}, gt{};' + chr(10) +
                  '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)
    return s + mask.replace('inline uint8_t', 'alignas(16) static uint8_t') + chr(10)


rd.vector_shim = _shim
import recomp_closure
# (the culler's second form, never taken here - +100 is 0 - converts with the SDK's unsigned-to-float helper)
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
SDK_INTRINSICS = 'D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h'
N_IN = 1 + 3 + 3 + 1 + 4 + 1 + 2 + 3 + 1 + 12
MAIN = r'''
static const uint32_t Q=0x40800000, C=0x40801000, O=0x40801400, AR=0x40801800, BD=0x40801C00;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}

int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 for(int k=0;k<4;k++) put(0x828609E0+4*k,0x7F800000);
 FILE*f=fopen(argv[2],"r"); unsigned w[NIN_];
 while(1){ int n=0; for(int i=0;i<NIN_;i++) n+=fscanf(f,"%x",&w[i]); if(n!=NIN_)break;
  rt_reset(); for(uint32_t a=Q;a<BD+0x100;a+=4) store(a,0);
  for(int j=0;j<3;j++){ store(Q+144+4*j,w[1+j]); store(Q+160+4*j,w[4+j]); }
  store(Q+176,w[7]); store(Q+208,w[7]);
  Context c=fresh(); c.r3.u64=Q; c.r4.u64=C;
  try{ if(w[0]) sub_CAPS_(c,nullptr); else sub_BALL_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  store(C+84,w[8]); store(C+88,w[9]); store(C+92,w[10]); store(C+96,w[11]); *pointer(C+100)=0;
  store(O+4,AR); store(AR+12,BD); store(AR+16,w[13]); store(AR+20,w[14]);
  for(int j=0;j<3;j++) store(BD+48+4*j,w[15+j]); store(BD+60,0); store(BD+8,w[18]);
  for(int k=0;k<4;k++){ for(int j=0;j<3;j++) store(O+16+16*k+4*j,w[19+3*k+j]); store(O+28+16*k,k==3?0x3F800000u:0u); }
  for(int o=0;o<48;o+=4) printf("%08x ",load(C+o)); printf("%08x ",load(C+80));
  c=fresh(); c.r3.u64=C; c.r4.u64=O; c.r5.u64=w[12];
  try{ sub_CULL_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  printf("%d\n",c.r3.u32&0xFF); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--sdk', default=SDK_INTRINSICS, help='the recomp SDK intrinsics.h (VectorMaskR)')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    ap.add_argument('--seed', type=int, default=62425)
    ap.add_argument('--points', action='store_true', help='only balls of radius not above 0 (the point volume)')
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    cull = addr(a.tool, 'F-62425')
    ball = addr(a.tool, 'F-36470')
    caps = addr(a.tool, 'F-21683')
    rnd = random.Random(a.seed)
    cases = []
    while len(cases) < a.cases:
        capsule = rnd.random() < 0.6 and not a.points
        sc = rnd.choice([0.5, 3.0, 20.0])
        A = [rnd.uniform(-1, 1) * sc for _ in range(3)]
        d = [rnd.gauss(0, 1) for _ in range(3)]
        ln = rnd.choice([0.0, 0.5, 2.0, 8.0]) * sc if capsule and rnd.random() < 0.95 else 0.0
        dn = math.sqrt(sum(x * x for x in d)) or 1.0
        B = [A[j] + d[j] / dn * ln for j in range(3)] if capsule else A
        r = f32(rnd.choice([0.3, 1.0, rnd.uniform(0.05, 4.0)]))
        if a.points:
            r = f32(rnd.choice([0.0, -0.0, -1.0, -rnd.uniform(0.01, 3.0)]))
        br = f32(rnd.choice([0.5, 2.0, rnd.uniform(0.1, 6.0)]))
        # the bound's sphere: its centre about the query's surface (the sum of the radii from a point of the axis),
        # inside, or far
        t = rnd.uniform(-0.3, 1.3)
        on = [A[j] + (B[j] - A[j]) * t for j in range(3)]
        e = [rnd.gauss(0, 1) for _ in range(3)]
        en = math.sqrt(sum(x * x for x in e)) or 1.0
        dist = ((0.0 if a.points else r) + br) * rnd.choice([rnd.uniform(0.98, 1.02), rnd.uniform(0.999, 1.001), rnd.uniform(0, 1), rnd.uniform(1, 3)])
        wc = [on[j] + e[j] / en * dist for j in range(3)]
        if rnd.random() < 0.25:
            rows = [[1.0, 0, 0], [0, 1.0, 0], [0, 0, 1.0]]
        else:
            q = [rnd.gauss(0, 1) for _ in range(4)]
            s = math.sqrt(sum(x * x for x in q))
            w_, x, y, z = (c / s for c in q)
            rows = [[1 - 2 * (y * y + z * z), 2 * (x * y + w_ * z), 2 * (x * z - w_ * y)],
                    [2 * (x * y - w_ * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w_ * x)],
                    [2 * (x * z + w_ * y), 2 * (y * z - w_ * x), 1 - 2 * (x * x + y * y)]]
        pos = [rnd.uniform(-2, 2) * sc for _ in range(3)]
        # the bound's centre in the object's frame: rows . (world - place)
        lc = [sum(rows[i][k] * (wc[k] - pos[k]) for k in range(3)) for i in range(3)]
        include = rnd.choice([0x83A, 0xFFFFFFFF, 0x812])
        types = rnd.choice([0xFFFFFFFF] * 5 + [0x4000])
        exclude = rnd.choice([0] * 8 + [0x100])
        states = rnd.choice([7] * 6 + [1, 4])
        bit = rnd.choice([1, 2, 4])
        arch = rnd.choice([0x189, 0x185, 0x1100, 0x20, 0x2, 0x900])
        ainc = rnd.choice([0xFFFFFFFF, 0xFFFFBFFF, 0x184E])
        row = ([int(capsule)] + [fb(f32(v)) for v in A + B] + [fb(r), include, exclude, types, states, bit, arch, ainc] +
               [fb(f32(v)) for v in lc] + [fb(br)] + [fb(f32(v)) for rr in rows for v in rr] + [fb(f32(v)) for v in pos])
        assert len(row) == N_IN
        cases.append(row)
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%08x' % x for x in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    main_src = MAIN.replace('NIN_', str(N_IN)).replace('CULL_', cull).replace('BALL_', ball).replace('CAPS_', caps)
    exe = rd.build(idx, ['sub_' + cull, 'sub_' + ball, 'sub_' + caps], main_src, work, 'cull', image=img)
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-500:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = [' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    nl = chr(10)
    (a.out_dir / ('instance_cull_point_ref.txt' if a.points else 'instance_cull_ref.txt')).write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split()[-1] for o in out]
    print(len(cases), 'cases;', sum(x == '1' for x in res), 'kept')


if __name__ == '__main__':
    main()
