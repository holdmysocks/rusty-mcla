"""The hull's point-near-an-edge test F-96748 run offline (generated code unchanged, vector mode, the recomp's -msse4.1
-mssse3): v1 = a, v2 = b the edge's ends, v3 the point, v4 the squared radius (splat, its w lane garbage: the routine
reads x, y, z); r3 out (1 when the point is within the radius of the segment).

Cases: random edges 1 mm .. 10 m long, points before, along and past it, degenerate edges (a == b, or shorter than
1e-6), the radius drawn round the true distance (the answers split about evenly) and at the exact float squared distance.

Writes <out-dir>/beside_edge_ref.txt, one case a line: "a xyz b xyz p xyz r2 -> 0|1" (hex)."""
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
    fe = addr(a.tool, 'F-96748')
    rnd = random.Random(96748)
    cases = []
    for _ in range(10000):
        p0 = [rnd.uniform(-50, 50) for _ in range(3)]
        k = rnd.random()
        if k < 0.1:
            p1 = list(p0)
        elif k < 0.15:
            p1 = [x + rnd.uniform(-1e-7, 1e-7) for x in p0]
        else:
            L = 10 ** rnd.uniform(-3, 1)
            d = [rnd.gauss(0, 1) for _ in range(3)]
            n = math.sqrt(sum(x * x for x in d)) or 1.0
            p1 = [x + L * y / n for x, y in zip(p0, d)]
        t = rnd.choice((rnd.uniform(-0.5, 1.5), rnd.uniform(0, 1), 0.0, 1.0))
        off = [rnd.gauss(0, 1) * 10 ** rnd.uniform(-3, 0) for _ in range(3)]
        p = [x + t * (y - x) + o for x, y, o in zip(p0, p1, off)]
        tt = max(0.0, min(1.0, t))
        q = [x + tt * (y - x) - z for x, y, z in zip(p0, p1, p)]
        dist2 = sum(x * x for x in q)
        r2 = rnd.choice((dist2 * rnd.uniform(0.5, 1.5), dist2 * rnd.uniform(0.99, 1.01), dist2))
        cases.append(p0 + p1 + p + [r2])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in cases)
    body = ('static const uint32_t C[][10]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){Context ctx{};ctx.r1.u32=0x2F00;' +
            'for(int k=0;k<3;k++){ctx.v1.u32[3-k]=c[k];ctx.v2.u32[3-k]=c[3+k];ctx.v3.u32[3-k]=c[6+k];ctx.v4.u32[3-k]=c[9];}' +
            'ctx.v1.u32[0]=ctx.v2.u32[0]=ctx.v3.u32[0]=0;ctx.v4.u32[0]=0xC0000000;' +
            ('sub_%s(ctx,nullptr);printf("%%u' % fe) + NL + '",ctx.r3.u32);}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fe], main_src, a.out_dir, 'beside_edge', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'beside_edge_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o == '1' for o in out), 'inside')


if __name__ == '__main__':
    main()
