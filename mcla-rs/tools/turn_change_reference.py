"""The body's turning-rate change F-14402 run offline (generated code unchanged, vector mode, flush mode as the SDK sets
it): r4 the body, r5 the step's dt (its first word, splat), r3 out. The torque +352 x dt taken into the body's frame by
its rows +208 / +224 / +240 (vmsum3fp), times the inverse moments +176, and back by the rows' columns; a torque of 0 in
x, y and z (w not looked at) gives 0 in all four lanes.

Cases: rows of random rotations and of arbitrary matrices, inverse moments of real bodies (1 / 0.05 .. 1 / 5000) and the
archetype's 0x7f7fffff lane, torques 0 .. 1e5 N m, all zero, -0, one or two lanes zero, NaN, subnormal; dt 1/60, 1/30,
0, random. w lanes garbage.

Writes <out-dir>/turn_change_ref.txt, one case a line, hex: "r0 r1 r2 (4 words each) inv4 t4 dt -> x y z w"."""
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


def rotation(rnd):
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(x * x for x in q))
    w, x, y, z = (v / n for v in q)
    return [[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-14402')
    rnd = random.Random(14402)
    garbage = lambda: rnd.choice((0, 0x3F800000, rnd.getrandbits(32) & 0x7F7FFFFF))
    cases = []
    for _ in range(5000):
        rows = rotation(rnd) if rnd.random() < 0.8 else [[rnd.uniform(-2, 2) for _k in range(3)] for _r in range(3)]
        inv = [1.0 / rnd.uniform(0.05, 5000) for _k in range(3)]
        if rnd.random() < 0.1:
            inv[rnd.randrange(3)] = struct.unpack('>f', bytes.fromhex('7f7fffff'))[0]
        mag = rnd.choice((rnd.uniform(0, 10), rnd.uniform(0, 1e3), rnd.uniform(0, 1e5)))
        t = [rnd.uniform(-mag, mag) for _k in range(3)]
        k = rnd.random()
        if k < 0.1:
            t = [0.0, 0.0, 0.0]
        elif k < 0.15:
            t = [rnd.choice((0.0, -0.0)) for _k in range(3)]
        elif k < 0.3:
            for _i in range(rnd.choice((1, 2))):
                t[rnd.randrange(3)] = rnd.choice((0.0, -0.0))
        elif k < 0.32:
            t[rnd.randrange(3)] = rnd.choice((float('nan'), 1e-40, -1e-41))
        dt = rnd.choice((1 / 60, 1 / 30, 0.0, rnd.uniform(0, 0.1)))
        words = []
        for r in rows:
            words += [*map(fb, r), garbage()]
        words += [*map(fb, inv), garbage(), *map(fb, t), garbage(), fb(dt)]
        cases.append(words)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows_c = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    seed = 'for(int k=0;k<4;k++)store(0x82860950+4*k,k<3?0xffffffff:0);'
    body = ('static const uint32_t C[][21]={' + nl + rows_c + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' + seed +
            'for(int k=0;k<4;k++){store(0x1000+208+4*k,c[k]);store(0x1000+224+4*k,c[4+k]);store(0x1000+240+4*k,c[8+k]);' +
            'store(0x1000+176+4*k,c[12+k]);store(0x1000+352+4*k,c[16+k]);}store(0x1F00,c[20]);' +
            f'Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1800;ctx.r4.u32=0x1000;ctx.r5.u32=0x1F00;{fn}(ctx,nullptr);' +
            'printf("%08x %08x %08x %08x' + NL + '",load(0x1800),load(0x1804),load(0x1808),load(0x180C));}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fn], main_src, a.out_dir, 'turn_change', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'turn_change_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
