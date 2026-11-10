"""The walker's sphere set F-59837 run offline for its three spheres (k = 0, 1, 2; mostly 2): the generated code whole and unchanged in vector mode
on flat guest memory (as ped_ahead_reference.py). r4 = T's matrix (0x1100: row 2 at +32, the place +48), r6 = the walker
(0x2000: +500 / +52 the proxy chain, +112, the MWC +384 lo / +388 carry), r7 = k, f2 the radius. Writes
<out-dir>/sphere2_ref.txt: the case's words (row 2 xyz, the place xyz, lo, carry, radius, k) '->' the sphere's centre
(+48 .. +56 of the sphere at the walker +944 + k x 1216), the radius (+64), the pending word (+20), the shape type (+32),
the include mask (+36) and the MWC after."""

import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    # (the vector unit flushes denormals in and out: the runtime's enableFlushMode sets the host's FTZ and DAZ, its
    # disableFlushMode clears them for the scalar unit; the shared shim leaves both empty)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    s = s.replace(fp, 'struct { void disableFlushMode() {_mm_setcsr(_mm_getcsr()&~0x8040u);} void enableFlushMode() {_mm_setcsr(_mm_getcsr()|0x8040u);}'
                  ' void disableFlushModeUnconditional() {_mm_setcsr(_mm_getcsr()&~0x8040u);}'
                  ' void enableFlushModeUnconditional() {_mm_setcsr(_mm_getcsr()|0x8040u);} } fpscr;', 1)
    return s


_shim = recomp_closure.shim
recomp_closure.shim = lambda scratch_end=0x3000: _shim(0x8000)
recomp_closure.vector_shim = vector_shim
NL = chr(92) + 'n'
recomp_closure.EXTRA += ('static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10) +
                         'static const uint32_t *K;' + chr(10) +
                         'static void ind(Context&ctx,uint32_t a){'
                         'printf(" I%08x",a);fflush(stdout);throw std::runtime_error("unexpected indirect call");}' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) ind(ctx,a)' + chr(10))
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


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
    fn = addr(a.tool, 'F-59837')
    rnd = random.Random(59837)
    cases = []
    for _ in range(4000):
        yaw = rnd.uniform(-3.2, 3.2)
        r2 = rnd.choice(([math.sin(yaw), 0.0, math.cos(yaw)], [rnd.uniform(-1, 1) for _ in range(3)], [1e-39, 0.0, -1e-39]))
        pl = rnd.choice(([rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)], [0.0, 0.0, 0.0], [1e-38, 0.5, -1e-38]))
        cases.append([fb(v) for v in r2 + pl] + [rnd.getrandbits(32), rnd.getrandbits(32), fb(rnd.choice((0.667, 2.0, rnd.uniform(0, 3)))), rnd.choice((2, 2, 0, 1))])
    W = len(cases[0])
    nl = chr(10)
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));K=c;'
            'for(int k=0;k<3;k++){store(0x1100+32+4*k,c[k]);store(0x1100+48+4*k,c[3+k]);}store(0x1100+60,0x3F800000);'
            'store(0x2000+384,c[6]);store(0x2000+388,c[7]);'
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r4.u32=0x1100;ctx.r6.u32=0x2000;ctx.r7.u32=c[9];uint32_t S=0x2000+944+1216*c[9];ctx.f2.f64=double(bf(c[8]));'
            'sub_%s(ctx,nullptr);' % fn +
            ('printf("%%08x %%08x %%08x %%08x %%08x %%08x %%08x %%08x %%08x%s",load(S+48),load(S+52),load(S+56),load(S+64),load(S+20),load(S+32),load(S+36),'
             'load(0x2000+384),load(0x2000+388));}' % (chr(92) + 'n',)) + nl)
    main_src = ('static float bf(uint32_t u){float f;memcpy(&f,&u,4);return f;}' + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], main_src, a.out_dir, 'sphere2', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sphere2_ref.txt').write_text(re.sub(r'(82[0-9a-f]{2})([0-9a-f]{4})', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
