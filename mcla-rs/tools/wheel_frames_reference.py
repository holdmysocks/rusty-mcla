"""Two of the wheel's frame routines run offline with what they call (generated code unchanged, vector mode, the recomp's
-msse4.1 -mssse3): the lean F-41945 (r3 three rows turned in place by the rotation about the axis at r4 by the angle f1,
built by F-10410) and the steering frame F-49981 (r3 out, the rotation taking r4 = up onto the axis at r5: F-81327 and
the axis-angle rows).

Cases (lean): random rows and axes, the six signed coordinate axes, non-unit axes, angles 0, +-pi, random in [-7, 7]
and tiny. Cases (frame): axes random, within 3.6 degrees of up or down either side of the limit, straight up and down,
and the six signed coordinate axes. No case has denormal intermediates (the recomp on x86 keeps denormals the VMX flushes).

Writes <out-dir>/wheel_frames_ref.txt: "L rows(12) axis(4) angle -> rows xyz (9)" and "F axis xyz -> rows xyz (9)" (hex)."""
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
    fl, ff = addr(a.tool, 'F-41945'), addr(a.tool, 'F-49981')
    rnd = random.Random(41945)

    def unit():
        v = [rnd.gauss(0, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v)) or 1.0
        return [x / n for x in v]

    def signed_axis():
        v = [0.0, 0.0, 0.0]
        v[rnd.randrange(3)] = rnd.choice((1.0, -1.0)) * rnd.choice((1.0, 1.0, rnd.uniform(0.1, 5)))
        return v

    lean = []
    for _ in range(6000):
        rows = [x for _ in range(3) for x in unit() + [rnd.choice((0.0, 1.0, rnd.uniform(-5, 5)))]]
        k = rnd.random()
        ax = signed_axis() if k < 0.3 else unit() if k < 0.8 else [x * rnd.uniform(0.01, 100) for x in unit()]
        ang = rnd.choice((0.0, math.pi, -math.pi, rnd.uniform(-7, 7), rnd.uniform(-7, 7), rnd.uniform(-1e-3, 1e-3)))
        lean.append(rows + ax + [0.0] + [ang])
    frame = []
    lim = math.acos(struct.unpack('>f', struct.pack('>I', 0x3f7f7cff))[0])
    for _ in range(4000):
        k = rnd.random()
        if k < 0.5:
            v = unit()
        elif k < 0.8:
            t = lim * rnd.uniform(0.9, 1.1)
            p = rnd.uniform(-math.pi, math.pi)
            v = [math.sin(t) * math.cos(p), math.cos(t), math.sin(t) * math.sin(p)]
            if rnd.random() < 0.5:
                v = [x * -1 for x in v]
        elif k < 0.9:
            v = [0.0, rnd.choice((1.0, -1.0)), 0.0]
        else:
            v = signed_axis()
            v = [x / max(abs(y) for y in v) for x in v]
        frame.append(v + [0.0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rl = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in lean)
    rf = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in frame)
    body = ('static const uint32_t L[][17]={' + nl + rl + nl + '};' + nl + 'static const uint32_t F[][4]={' + nl + rf + nl + '};' + nl +
            'for(auto&c:L){memset(objects,0,sizeof(objects));for(int i=0;i<12;i++)store(0x1000+4*i,c[i]);for(int i=0;i<4;i++)store(0x1040+4*i,c[12+i]);'
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1040;float f;memcpy(&f,&c[16],4);ctx.f1.f64=f;' +
            ('sub_%s(ctx,nullptr);' % fl) + 'for(int r=0;r<3;r++)for(int k=0;k<3;k++)printf("%08x ",load(0x1000+16*r+4*k));printf("' + NL + '");}' + nl +
            'for(auto&c:F){memset(objects,0,sizeof(objects));store(0x1044,0x3F800000);for(int i=0;i<4;i++)store(0x1080+4*i,c[i]);'
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1040;ctx.r5.u32=0x1080;' +
            ('sub_%s(ctx,nullptr);' % ff) + 'for(int r=0;r<3;r++)for(int k=0;k<3;k++)printf("%08x ",load(0x1000+16*r+4*k));printf("' + NL + '");}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fl, 'sub_' + ff], main_src, a.out_dir, 'wheel_frames', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(lean) + len(frame), (len(out), len(lean), len(frame))
    lines = ['L %s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(lean, out)]
    lines += ['F %s -> %s' % (' '.join('%08x' % fb(x) for x in c[:3]), o) for c, o in zip(frame, out[len(lean):])]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'wheel_frames_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lean), 'leans,', len(frame), 'frames')


if __name__ == '__main__':
    main()
