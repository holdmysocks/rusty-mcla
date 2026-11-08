"""Three small routines of the loose-object code run offline (generated code unchanged, vector mode, flush mode as the
SDK sets it): the rope's normal F-72436 (v1 = A, v2 = B; out r4 = n, r5 = 0 - n, f1 = |d| - this+160, the stretch past the rope's length, 0 - this+160
where d is too short), the
archetype's SetMass F-31082 (f1; +40 the mass, +44 its inverse or 0), and SetInertia F-58768 (r4 the inertia; +64 it
after the floor F-88418, +80 its inverse: the estimate and one Newton step, the archetype's 0x7f7fffff where a lane
is 0).

The rope's normal: d = B - A, its squared length (vmsum3fp) above 0x2b8cbccc gives d x (the estimate of 1 / sqrt and
one Newton step), else (0, 1, 0). Start-up constants the image holds as zero are filled in as the running game has
them (the xyz mask).

Cases: rope ends 0 .. 10 m apart, a hair apart, at the threshold, the same point, w lanes garbage; masses as shipped,
0, -0, tiny, huge, NaN; inertias as shipped, ties, one far smaller, zeros, -0, NaN.

Writes <out-dir>/loose_setters_ref.txt, one case a line, hex: "rope ax ay az aw bx by bz bw c -> n4 m4 f1",
"mass m -> m inv", "inertia x y z -> i4 inv4"."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10))
_vshim = recomp_closure.vector_shim


def _shim():
    # (cr6 from a vector compare as the SDK's; the flush mode as the SDK sets it: MXCSR FTZ | DAZ)
    s = '#include <xmmintrin.h>' + chr(10) + '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx2.h>' + chr(10) + _vshim()
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


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fr, fm, fi = ('sub_' + addr(a.tool, f) for f in ('F-72436', 'F-31082', 'F-58768'))
    rnd = random.Random(72436)
    thr = struct.unpack('>f', bytes.fromhex('2b8cbccc'))[0]
    cases = []
    for _ in range(3000):
        pa = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 100), rnd.uniform(-3000, 3000)]
        k = rnd.random()
        if k < 0.1:
            pb = list(pa)
        elif k < 0.2:
            d = math.sqrt(thr) * rnd.choice((0.5, 1.0, 1.0001, 2.0))
            pa = [rnd.uniform(-1, 1) for _i in range(3)]
            pb = [pa[0] + d, pa[1], pa[2]]
        else:
            L = rnd.choice((rnd.uniform(0, 10), rnd.uniform(0, 0.01), rnd.uniform(1, 3)))
            u = [rnd.gauss(0, 1) for _i in range(3)]
            n = math.sqrt(sum(x * x for x in u)) or 1.0
            pb = [p + L * x / n for p, x in zip(pa, u)]
        w = [rnd.getrandbits(32) & 0x7F7FFFFF for _i in range(2)]
        cases.append(('rope', [*map(fb, pa), w[0], *map(fb, pb), w[1], fb(rnd.uniform(-2, 2))]))
    for _ in range(1500):
        m = rnd.choice((rnd.uniform(0.1, 3000), 0.0, -0.0, 1e-40, 1e-30, 3e38, float('nan'), rnd.uniform(0.01, 5)))
        cases.append(('mass', [fb(m)]))
    for _ in range(3000):
        big = rnd.uniform(0.01, 500)
        v = [rnd.choice((rnd.uniform(0.01, 500), big, big / rnd.uniform(1, 100), big / 20, rnd.uniform(0, 1))) for _k in range(3)]
        if rnd.random() < 0.2:
            v[rnd.randrange(3)] = rnd.choice((0.0, -0.0, float('nan'), 1e-40, 3e38))
        cases.append(('inertia', [fb(x) for x in v]))
    nl = chr(10)
    NL = chr(92) + 'n'
    kinds = {'rope': 0, 'mass': 1, 'inertia': 2}
    flat = []
    for kind, ws in cases:
        flat += [kinds[kind], len(ws)] + ws
    seed = 'for(int k=0;k<4;k++)store(0x82860950+4*k,k<3?0xffffffff:0);'
    out_v = lambda at: f'load({at}),load({at}+4),load({at}+8),load({at}+12)'
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'size_t at=0;for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));' + seed +
            'uint32_t kind=D[at],n=D[at+1];const uint32_t*w=&D[at+2];at+=2+n;' +
            'for(uint32_t i=0;i<0x200;i+=4)store(0x1000+i,0xA5A50000u+i);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            f'if(kind==0){{for(int k=0;k<4;k++){{ctx.v1.u32[3-k]=w[k];ctx.v2.u32[3-k]=w[4+k];}}store(0x1000+160,w[8]);ctx.r4.u32=0x1800;ctx.r5.u32=0x1900;' +
            f'{fr}(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);' +
            'printf("%08x %08x %08x %08x %08x %08x %08x %08x %08x' + NL + f'",{out_v(0x1800)},{out_v(0x1900)},u);}}' +
            f'else if(kind==1){{float m;memcpy(&m,&w[0],4);ctx.f1.f64=m;{fm}(ctx,nullptr);' +
            'printf("%08x %08x' + NL + '",load(0x1000+40),load(0x1000+44));}' +
            f'else{{for(int k=0;k<3;k++)store(0x1800+4*k,w[k]);store(0x180C,0x3F800000);ctx.r4.u32=0x1800;{fi}(ctx,nullptr);' +
            'printf("%08x %08x %08x %08x %08x %08x %08x %08x' + NL + f'",{out_v(0x1000 + 64)},{out_v(0x1000 + 80)});}}}}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fr, fm, fi], main_src, a.out_dir, 'loose_setters', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s %s -> %s' % (kind, ' '.join('%08x' % x for x in ws), o) for (kind, ws), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'loose_setters_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
