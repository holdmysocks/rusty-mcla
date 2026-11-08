"""The ragdoll's vectors across an axis run offline (generated code unchanged, vector mode, the recomp's -msse4.1 -mssse3;
lane k of a register is the host's f32[3 - k]): F-51707 (v1 the axis, r3 one unit vector across it) and F-40242 (v1 the
axis, r3 and r4 two unit vectors across it, the second unit(n x u)).

Cases: random unit and non-unit axes, the axes and their negatives, lanes at 0.5 exactly and one ulp either side, tiny
(1e-15, 1e-12) and zero vectors, the w lane filled with garbage. No case has a denormal length squared: the recomp on x86
keeps denormals that the Xbox's vector unit (and the port's V4) flushes, so such a case would compare the harness, not
the original. NaN sign bits differ the same way (x86's default NaN is negative); the tests compare NaN as NaN.

Writes <out-dir>/across_ref.txt, one case a line: "axis xyzw -> F-51707's 4 words | F-40242's 4 + 4 words" (hex)."""
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
    f1, f2 = addr(a.tool, 'F-51707'), addr(a.tool, 'F-40242')
    rnd = random.Random(51707)
    half = 0.5
    edge = (struct.unpack('>f', struct.pack('>I', 0x3effffff))[0], half, struct.unpack('>f', struct.pack('>I', 0x3f000001))[0])
    cases = []
    for i in range(8000):
        k = rnd.random()
        if k < 0.5:
            v = [rnd.gauss(0, 1) for _ in range(3)]
            n = math.sqrt(sum(x * x for x in v)) or 1.0
            v = [x / n for x in v]
        elif k < 0.7:
            v = [rnd.gauss(0, 1) * 10 ** rnd.uniform(-4, 3) for _ in range(3)]
        elif k < 0.8:
            v = [0.0, 0.0, 0.0]
            v[rnd.randrange(3)] = rnd.choice((1.0, -1.0))
        elif k < 0.95:
            v = [rnd.uniform(-1, 1) for _ in range(3)]
            v[rnd.randrange(2)] = rnd.choice((1, -1)) * rnd.choice(edge)
        else:
            v = [rnd.choice((0.0, -0.0, 1e-15, -1e-12)) for _ in range(3)]
        cases.append(v + [rnd.choice((0.0, 1.0, rnd.uniform(-1e3, 1e3)))])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in cases)
    call = ('memset(objects,0,sizeof(objects));{Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1010;'
            'for(int k=0;k<4;k++)ctx.v1.u32[3-k]=c[k];sub_%s(ctx,nullptr);}')
    body = ('static const uint32_t C[][4]={' + nl + rows + nl + '};' + nl + 'for(auto&c:C){' +
            (call % f1) + 'for(int i=0;i<4;i++)printf("%08x ",load(0x1000+4*i));printf("| ");' +
            (call % f2) + 'for(int i=0;i<8;i++)printf("%08x ",load(0x1000+4*i));printf("' + NL + '");}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + f1, 'sub_' + f2], main_src, a.out_dir, 'across', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'across_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
