"""The glass draw's window states, tints and clip plane F-50822 run offline on the generated code unchanged.

F-50822 builds WindowDmgVal (9 floats), TintColors and BumpedTintColors (9 vectors each) and the side windows' clip
plane, and hands them to the shader variables (F-56275, F-73216, F-34251 - stubbed to print what they are handed) and
to gCarConstantBuffer (F-28069 puts the plane at +512, F-92230 splats the states at +64 + 16 i - both run; the upload
F-75931 is stubbed to print the buffer's rows). The motorcycle test F-54299 is stubbed to answer from the case.

Draw object G at 0x1000 (the original's offsets): +20 -> the car, +123 the in-car byte, +1060 -> the variables' handle,
+5348 + 80 i the window pieces' hit sums (i 0 .. 8), +6478 the game's no-damage byte. Car at 0x3000: +80 -> 0x3400
(+41 the damage gate byte), +76 -> 0x3500 (+96 -> 0x3600: +180 the window clip height), +132 -> the record P at 0x3800
(P + 64 + 224 .. + 288: headlight, taillight, bumped headlight, bumped taillight glass and the window tint), +144 / +208
the matrices (the thread byte picks +208). r4: the plane's flag byte.

Writes <out-dir>/glass_vars_ref.txt, one case a line: "in: <words> -> out: <words>"; in = the four sums, the five
record vectors (20), the +144 and +208 matrices (32), h, then G+123 G+6478 gate bike flag thread; out = WindowDmgVal (9),
TintColors (36), BumpedTintColors (36), the plane (4) (the buffer's rows +64 + 16 i, the states splatted, checked
by the script).
Test `damagemap::tests::glass_vars_match_the_original`."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import recomp_leaf

recomp_closure.EXTRA += 'static int bike;\n#define ctr r2\n#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("unexpected indirect call")\n'
recomp_closure.shim = lambda: recomp_leaf.shim(0x8000)
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def mat(r, place=50.0):
    a, t, s = r.uniform(-3, 3), r.uniform(-0.6, 0.6), r.uniform(-0.6, 0.6)
    ca, sa, ct, st, cs, ss = math.cos(a), math.sin(a), math.cos(t), math.sin(t), math.cos(s), math.sin(s)
    Y = [[ca, 0, -sa], [0, 1, 0], [sa, 0, ca]]
    Z = [[ct, st, 0], [-st, ct, 0], [0, 0, 1]]
    X = [[1, 0, 0], [0, cs, ss], [0, -ss, cs]]
    mul = lambda p, q: [[sum(p[i][k] * q[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    m = mul(mul(X, Z), Y)
    k = r.choice((1.0, 1.0, r.uniform(0.5, 1.5)))  # (a scaled up row now and then: the plane takes it as it is)
    return [m[0][0], m[0][1], m[0][2], r.uniform(-1, 1), k * m[1][0], k * m[1][1], k * m[1][2], r.uniform(-1, 1),
            m[2][0], m[2][1], m[2][2], r.uniform(-1, 1), r.uniform(-place, place), r.uniform(-5, 5), r.uniform(-place, place), 1]


# (the vector unit's constants a static initialiser fills; the thread byte's page)
COMMON = ('memset(objects,0,0x7000);'
          'for(int k=0;k<4;k++){store(0x827D85D0+16*k+4*k,0x3f800000);store(0x82860950+4*k,k<3?0xffffffff:0);'
          'store(0x82860930+4*k,k<3?0:0x3f800000);}'
          'store(0x5800,0x5840);')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-50822')
    rnd = random.Random(50822)
    nl = chr(10)

    def summ():
        return rnd.choice((rnd.uniform(0, 10), 4.0, 6.0, 4.0 - 2 ** -21, 6.0 - 2 ** -21, 0.0, -1.0, math.nan, math.inf, rnd.uniform(3.9, 6.1)))

    def vec():
        return rnd.choice(([0.0, 0.0035, 0.005, 0.372], [rnd.uniform(0, 1) for _ in range(4)], [0.0, 0.0, 0.0, rnd.choice((0.0, 0.3922, 0.1961))]))

    cases = []
    for k in range(a.cases):
        sums = [summ() for _ in range(4)]
        recs = sum((vec() for _ in range(5)), [])
        mats = mat(rnd) + mat(rnd)
        h = rnd.choice((-1.0, 0.7675, 1.05, rnd.uniform(0.5, 1.3)))
        ints = [rnd.choice((0, 0, 1)), rnd.choice((0, 0, 1)), rnd.choice((1, 1, 0)), rnd.choice((0, 0, 1)), rnd.choice((1, 1, 0, 0x100)), rnd.choice((0, 1))]
        cases.append((sums + recs + mats + [h], ints))
    rows = ',\n'.join('{%s}' % ','.join(['0x%08x' % fb(float(x)) for x in c] + ['%du' % x for x in i]) for c, i in cases)
    nf = len(cases[0][0])
    setup = (COMMON + 'store(0x1000+20,0x3000);store(0x1000+1060,0x3C00);'
             'for(int i=0;i<9;i++)store(0x1000+5348+80*i,i<4?c[i]:0x7fc00000);'
             'for(int k=0;k<20;k++)store(0x3800+64+224+4*k,c[4+k]);'
             'for(int k=0;k<16;k++){store(0x3000+144+4*k,c[24+k]);store(0x3000+208+4*k,c[40+k]);}'
             'store(0x3000+80,0x3400);store(0x3000+76,0x3500);store(0x3500+96,0x3600);store(0x3600+180,c[56]);store(0x3000+132,0x3800);'
             '*pointer(0x1000+123)=c[%d];*pointer(0x1000+6478)=c[%d];*pointer(0x3400+41)=c[%d];bike=c[%d];*pointer(0x5840+72)=c[%d];'
             % (nf, nf + 1, nf + 2, nf + 3, nf + 5))
    pr = lambda base, n: 'for(int k=0;k<%d;k++)printf("%%08x ",load(%s+4*k));' % (n, base)
    stubs = {
        'sub_' + addr(a.tool, 'F-54299'): 'ctx.r3.u64=bike;',
        'sub_' + addr(a.tool, 'F-56275'): 'printf("V ");' + pr('ctx.r4.u32', 9),
        'sub_' + addr(a.tool, 'F-73216'): 'printf("A ");' + pr('ctx.r4.u32', 36),
        'sub_' + addr(a.tool, 'F-34251'): 'printf("B ");' + pr('ctx.r4.u32', 36),
        'sub_' + addr(a.tool, 'F-75931'): 'printf("C ");' + pr('ctx.r3.u32+64', 36) + pr('ctx.r3.u32+512', 4),
    }
    body = (
        'static const uint32_t K[][%d]={' % (nf + 6) + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
        'for(auto&cc:K){const uint32_t*c=cc;' + setup +
        'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u64=c[%d];ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' % (nf + 4) + fn +
        '(ctx,nullptr);printf("' + chr(92) + 'n");}}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], body, a.out_dir, 'glassvars', stubs=stubs, vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    lines = []
    for (c, i), o in zip(cases, out):
        # (the order the stubs print in: buffer C, then V, A, B)
        parts = {}
        key = None
        for w in o.split():
            if w in ('V', 'A', 'B', 'C'):
                key = w
                parts[key] = []
            else:
                parts[key].append(w)
        assert [len(parts[x]) for x in 'VABC'] == [9, 36, 36, 40], o[:200]
        # (the buffer's rows +64 + 16 i: WindowDmgVal[i] in all four lanes - checked here, only the plane written out)
        assert parts['C'][:36] == [w for w in parts['V'] for _ in range(4)], o[:200]
        lines.append(' '.join('%08x' % fb(float(x)) for x in c) + ' ' + ' '.join(str(x) for x in i) + ' -> ' +
                     ' '.join(parts['V'] + parts['A'] + parts['B'] + parts['C'][36:]))
    (a.out_dir / 'glass_vars_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
