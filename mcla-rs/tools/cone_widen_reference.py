"""The contact normal's cone routines run offline (generated code unchanged, vector mode, flush mode as the SDK sets it):
the widen F-12753 (r3 the cone's cosine in memory, v1 the axis, v2 the direction to hold; v1 out, the cosine written
back) and the clamp F-99162 (r3 the cosine, v1 the normal, v2 the axis; v1 out), with their arc cosine / cosine
helpers unchanged.

Cases as F-29567 makes them and round them: unit axes and directions, the direction from the same to the opposite of
the axis, cosines 1 (the face's own cone), at, just above and just below the 0.9998 switch, random, -1 .. 1, the
normal inside, on and outside the cone, unnormalised vectors, w lanes garbage.

Writes <out-dir>/cone_widen_ref.txt, one case a line, all words hex: "widen cos ax ay az aw ox oy oz ow -> x y z w cos"
and "clamp cos nx ny nz nw ax ay az aw -> x y z w"."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10) +
                         '#define REX_QUERY_TIMEBASE() 1' + chr(10))
_vshim = recomp_closure.vector_shim
# (the SDK's own vector helpers the generated code calls, taken from its intrinsics.h as they are: the unsigned convert
# of vcuxwfp128, the saturating convert of vcfpsxws128)
SDK = Path('D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h')


def sdk_helpers(names):
    text = SDK.read_text()
    out = []
    for n in names:
        m = re.search(r'^inline [\w:]+ ' + n + r'\(.*?^}\n', text, re.M | re.S)
        assert m, n
        out.append(m.group(0))
    return '#include <simde/x86/avx2.h>\n#include <climits>\nnamespace rex { namespace ppc {\n' + ''.join(out) + '} }\n'


recomp_closure.EXTRA += sdk_helpers(['simde_mm_cvtepu32_ps_', 'simde_mm_vctsxs'])


def _shim():
    # (avx for vpermwi's permute; cr6 from a vector compare as the SDK's; the flush mode as the SDK sets it: MXCSR FTZ | DAZ)
    s = ('#include <xmmintrin.h>' + chr(10) + '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx2.h>' + chr(10) +
         _vshim())
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fl = 'struct { void disableFlushMode() {} void enableFlushMode() {}'
    assert s.count(fl) == 1
    return s.replace(fl, 'struct { void disableFlushMode() { _mm_setcsr(_mm_getcsr() & ~0x8040); }'
                         ' void enableFlushMode() { _mm_setcsr(_mm_getcsr() | 0x8040); }', 1)


recomp_closure.vector_shim = _shim


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(rnd):
    while True:
        v = [rnd.gauss(0, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if n > 1e-3:
            return [x / n for x in v]


def towards(rnd, a, angle):
    # (a unit vector at `angle` from a)
    p = unit(rnd)
    d = sum(x * y for x, y in zip(p, a))
    p = [x - d * y for x, y in zip(p, a)]
    n = math.sqrt(sum(x * x for x in p)) or 1.0
    return [math.cos(angle) * y + math.sin(angle) * x / n for x, y in zip(p, a)]


def cosine(rnd):
    sw = struct.unpack('>f', bytes.fromhex('3f7ff972'))[0]
    return rnd.choice((1.0, 1.0, sw, struct.unpack('>f', bytes.fromhex('3f7ff971'))[0], struct.unpack('>f', bytes.fromhex('3f7ff973'))[0],
                       rnd.uniform(0.9998, 1.0), rnd.uniform(0.5, 1.0), rnd.uniform(-1, 1), rnd.uniform(0.99, 1.0), -1.0, 0.0))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fw, fc = 'sub_' + addr(a.tool, 'F-12753'), 'sub_' + addr(a.tool, 'F-99162')
    rnd = random.Random(12753)
    cases = []
    for _ in range(6000):
        kind = rnd.random() < 0.5
        c = cosine(rnd)
        ax = unit(rnd)
        ang = rnd.choice((rnd.uniform(0, math.pi), rnd.uniform(0, 0.1), rnd.uniform(0, 0.03), 0.0, math.pi, math.acos(max(-1, min(1, c)))))
        o = towards(rnd, ax, ang)
        if rnd.random() < 0.15:
            s = rnd.uniform(0.98, 1.02)
            o = [x * s for x in o]
        if rnd.random() < 0.05:
            o = [-x for x in ax]
        w1, w2 = (rnd.choice((0, 0x3F800000, rnd.getrandbits(32) & 0x7F7FFFFF)) for _k in range(2))
        cases.append((0 if kind else 1, fb(c), *map(fb, ax), w1, *map(fb, o), w2))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    body = ('static const uint32_t C[][10]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000,c[1]);Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            'for(int k=0;k<4;k++){ctx.v1.u32[3-k]=c[2+k];ctx.v2.u32[3-k]=c[6+k];}' +
            f'if(c[0]==0){fw}(ctx,nullptr);else {fc}(ctx,nullptr);' +
            'printf("%08x %08x %08x %08x %08x' + NL + '",ctx.v1.u32[3],ctx.v1.u32[2],ctx.v1.u32[1],ctx.v1.u32[0],load(0x1000));}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fw, fc], main_src, a.out_dir, 'cone_widen', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for c, o in zip(cases, out):
        o = o.split()
        name = 'widen' if c[0] == 0 else 'clamp'
        res = o if c[0] == 0 else o[:4]
        lines.append('%s %s -> %s' % (name, ' '.join('%08x' % x for x in c[1:]), ' '.join(res)))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'cone_widen_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
