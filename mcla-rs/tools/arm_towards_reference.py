"""The junction arm of a point F-97162 run offline: unchanged generated code (vector mode) on flat guest memory. The junction J
(r3) holds the arm count (u16 at +48), the arm array (+44 -> 16 bytes per arm: x y z w, raw, not unit) and the centre (vec4 at
+80, zero here: the port is called with the direction from the centre); r4 = the point (vec4). The call flattens (y = 0) and
normalises both with the vector unit (1 / sqrt estimate, one Newton step) and returns the index of the largest dot product
(the first of equals, strictly greater than -FLT_MAX), -1 with no arm.
Cases: random arms of any length and slope, the crossing's axes and diagonals (exact ties), repeated arms, points on the
bisectors.
Writes <out-dir>/arm_towards_ref.txt, one case a line: "n arms(x y z w each) point(x y z w) -> index" as hex words (index
decimal, -1 for none)."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


import recomp_closure
# (vupkd3d128 writes the SDK's global vTemp; vrlimi128 uses simde_mm_permute_ps)
recomp_closure.EXTRA += (
    'typedef VReg PPCVRegister;\n#define simde_mm_permute_ps(a,imm) simde_mm_shuffle_ps(a,a,imm)\n'
    'namespace rex { namespace ppc {\n'
    'inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {\n'  # the SDK's intrinsics.h
    '  simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));\n'
    '  simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8);\n'
    '  xmm0 = simde_mm_srli_epi32(xmm0, 31);\n'
    '  xmm0 = simde_mm_add_epi32(xmm0, xmm1);\n'
    '  xmm0 = simde_mm_srai_epi32(xmm0, 8);\n'
    '  xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000));\n'
    '  simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);\n'
    '  return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1));\n'
    '} } }\n')
_vector_shim = recomp_closure.vector_shim


def _shim():
    # the condition register's vector compare (the SDK's Condition::setFromMask) and its summary-overflow bit
    s = _vector_shim()
    old = 'struct Condition {\n    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    return s.replace(old, '#include <simde/x86/sse4.1.h>\nstruct Condition {\n    bool so{}, eq{}, lt{}, gt{};\n'
                     '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)


recomp_closure.vector_shim = _shim


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    ad = addr(a.tool, 'F-97162')
    rnd = random.Random(97162)

    def rv(scale=1.0, flat=False):
        y = 0.0 if flat else rnd.uniform(-0.5, 0.5) * scale
        return [rnd.uniform(-1, 1) * scale, y, rnd.uniform(-1, 1) * scale, 0.0]

    cases = []
    for c in range(a.cases):
        k = rnd.random()
        if k < 0.35:
            n = rnd.randint(1, 6)
            arms = [rv(10 ** rnd.uniform(-1, 2)) for _ in range(n)]
            pt = rv(10 ** rnd.uniform(-1, 2))
        elif k < 0.7:
            # a crossing: axes with an off-axis tilt, points on the bisectors and near them
            n = rnd.choice((3, 4, 4, 4, 5))
            base = rnd.uniform(0, 2 * math.pi) if rnd.random() < 0.5 else rnd.choice((0.0, math.pi / 2, 0.3, 1.0))
            step = 2 * math.pi / n
            sc = rnd.choice((1.0, 1.0, 7.5, 0.3))
            arms = [[math.sin(base + i * step) * sc, 0.0, math.cos(base + i * step) * sc, 0.0] for i in range(n)]
            ang = base + (rnd.randrange(n) + 0.5) * step + rnd.choice((0.0, 0.0, 1e-7, -1e-7, 1e-5))
            r = rnd.choice((1.0, 3.0, 12.5, rnd.uniform(0.1, 50)))
            pt = [math.sin(ang) * r, rnd.choice((0.0, 0.0, 1.5)), math.cos(ang) * r, 0.0]
        else:
            n = rnd.randint(2, 6)
            arms = [rv(rnd.choice((1.0, 20.0)), flat=True) for _ in range(n)]
            if rnd.random() < 0.4:
                arms[rnd.randrange(n)] = list(arms[0])
            pt = rv(rnd.choice((1.0, 100.0)))
        cases.append((n, [[f32(x) for x in v] for v in arms], [f32(x) for x in pt]))
    cases.append((0, [], [1.0, 0.0, 0.0, 0.0]))
    cases.append((2, [[0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]], [1.0, 0.0, 0.0, 0.0]))
    cases.append((2, [[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]], [0.0, 0.0, 0.0, 0.0]))
    nl = chr(10)
    NL = chr(92) + 'n'
    W = 6 * 4 + 4 + 1
    rows = []
    for n, arms, pt in cases:
        words = [n] + [fb(x) for v in arms for x in v] + [0] * (4 * (6 - n)) + [fb(x) for x in pt]
        rows.append('{%s},' % ','.join('0x%08x' % w for w in words))
    body = ('static const uint32_t C[][%d]={' % (1 + 24 + 4) + nl + nl.join(rows) + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000+44,0x1200);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;' +
            'store(0x1000+48,uint32_t(c[0])<<16);' +
            'for(int k=0;k<24;k++)store(0x1200+4*k,c[1+k]);for(int k=0;k<4;k++)store(0x1100+4*k,c[25+k]);' +
            ('sub_%s(ctx,nullptr);printf("%%d%s",int(ctx.r3.s32));}' % (ad, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.out_dir, 'arm_towards', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (n, arms, pt), o in zip(cases, out):
        ws = [fb(x) for v in arms for x in v] + [fb(x) for x in pt]
        lines.append('%d %s -> %s' % (n, ' '.join('%08x' % w for w in ws), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that would read as addresses of the original are split by '_'; the test removes it)
    (a.out_dir / 'arm_towards_ref.txt').write_text(re.sub(r'(82[0-9a-f]{2})([0-9a-f]{4})', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases,', sum(o == '-1' for o in out), 'without an arm')


if __name__ == '__main__':
    main()
