"""The shortest-arc rotation F-68966 (the ragdoll's link re-seat; it tails into the axis-angle rows F-28465) run offline
(generated code unchanged, vector mode, the recomp's -msse4.1 -mssse3; lane k of a register is the host's f32[3 - k]):
v1 = from, v2 = to, r3 the three rows out (16 bytes each).

Cases: random directions and lengths (1e-3 .. 1e3), near-parallel and anti-parallel pairs, equal ones, a zero vector, the
w lanes filled with garbage.

Writes <out-dir>/shortest_arc_ref.txt, one case a line: "from xyzw to xyzw -> 12 words (rows, lanes 0..3)" (hex)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure

_vshim = recomp_closure.vector_shim


def _shim():
    # (the SDK's own rex::ppc::simde_mm_cvtepu32_ps_, include/rex/ppc/intrinsics.h, for vcuxwfp128; cr6 from a vector compare)
    cvt = ('namespace rex { namespace ppc { inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {'
           'simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));'
           'simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8); xmm0 = simde_mm_srli_epi32(xmm0, 31);'
           'xmm0 = simde_mm_add_epi32(xmm0, xmm1); xmm0 = simde_mm_srai_epi32(xmm0, 8);'
           'xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000)); simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);'
           'return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1)); }'
           # (vperm, as the SDK's: byte index 15 - (c & 15), from b where bit 4 of c is set)
           'inline simde__m128i simde_mm_perm_epi8_(simde__m128i a, simde__m128i b, simde__m128i c) {'
           'simde__m128i d = simde_mm_set1_epi8(0xF); simde__m128i e = simde_mm_sub_epi8(d, simde_mm_and_si128(c, d));'
           'return simde_mm_blendv_epi8(simde_mm_shuffle_epi8(a, e), simde_mm_shuffle_epi8(b, e), simde_mm_slli_epi32(c, 3)); } } }')
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx.h>' + chr(10) + cvt + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = _shim


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fs = addr(a.tool, 'F-68966')
    rnd = random.Random(68966)

    def unit():
        v = [rnd.gauss(0, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v)) or 1.0
        return [x / n for x in v]

    cases = []
    for _ in range(8000):
        f = [x * 10 ** rnd.uniform(-3, 3) for x in unit()]
        k = rnd.random()
        if k < 0.4:
            t = [x * 10 ** rnd.uniform(-3, 3) for x in unit()]
        elif k < 0.6:
            s = rnd.choice((1, -1)) * 10 ** rnd.uniform(-1, 1)
            t = [s * x + rnd.uniform(-1, 1) * 10 ** rnd.uniform(-9, -2) for x in f]
        elif k < 0.7:
            t = list(f)
        elif k < 0.75:
            t = [0.0, 0.0, 0.0]
        else:
            t = unit()
            f = unit()
        cases.append(f + [rnd.choice((0.0, 1.0, rnd.uniform(-1e3, 1e3)))] + t + [rnd.choice((0.0, 1.0, rnd.uniform(-1e3, 1e3)))])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in cases)
    body = ('static const uint32_t C[][8]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            'for(int k=0;k<4;k++){ctx.v1.u32[3-k]=c[k];ctx.v2.u32[3-k]=c[4+k];}' +
            ('sub_%s(ctx,nullptr);for(int i=0;i<12;i++)printf("%%08x ",load(0x1000+4*i));printf("' % fs) + NL + '");}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fs], main_src, a.out_dir, 'shortest_arc', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'shortest_arc_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
