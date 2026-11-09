"""The semi's three constraints as the original makes them, run offline (generated code unchanged, vector mode, flush
mode as the SDK sets it):

A. F-15252 (a cab or trailer made active): its partner (the pool lookup F-18069, stubbed to the case's second car),
   each car's hitch (tune +168 -> +204 -> +96, read as is) through its instance's matrix (instance +16 rows, +64 place;
   car +112 is the instance while car +500 bit 0x20 is clear) with w = 1 (vmsum4fp); the second pair the hitches
   + 10 x row 2, the third + 10 x row 1 (vmulfp, then vaddfp). AddPairConstraint F-82928 is stubbed to log what it is
   handed - both world points, the length f1, the instances and the parts - and to hand back a constraint, which the
   routine then sets: the mode F-95940 (+200), +320 = 0, +328 = -1 and, on the first, +164 = 0.6. F-27121 ahead and
   the activation F-17857 / F-10339 at the end are stubbed.
B. The constraint's init F-44049 (r4 / r7 the instances, r5 / r8 the world points, r6 / r9 the parts, f1 the length)
   with its world-to-instance step unchanged: +24 / +28 the instances, +188 / +192 the parts, +80 / +96 the points in
   each instance's frame (the point less the place, into the rows; with the bound a composite, type 12, then into the
   part's matrix, bound +132, 64 bytes a part), +112 / +128 = 0, +324 = -1, +160 the length.

Cases: random rotations and city places (+-3000 m), hitches as a cab's and a trailer's and random, parts 0 / 1, bound
types 12 and 2, part matrices the identity and random.

Writes <out-dir>/semi_ropes_ref.txt: "pair <case words> -> <three calls: pA4 pB4 f1 iA iB partA partB> | <three
constraints: +164 +320 +328 +200>" and "init <case words> -> +24 +28 +188 +192 a4 b4 +112_4 +128_4 +324 +160"."""
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
                         'static uint32_t K;' + chr(10))
_vshim = recomp_closure.vector_shim


def _shim():
    # (cr6 from a vector compare as the SDK's; the flush mode as the SDK sets it: MXCSR FTZ | DAZ; the two cars, their
    # tunes, the constraints and a stack: scratch 0x1000 .. 0x10000)
    s = '#include <xmmintrin.h>' + chr(10) + '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx2.h>' + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fl = 'void disableFlushMode() {} void enableFlushMode() {}'
    assert s.count(fl) == 1
    s = s.replace(fl, 'void disableFlushMode() { _mm_setcsr(_mm_getcsr() & ~0x8040); }'
                      ' void enableFlushMode() { _mm_setcsr(_mm_getcsr() | 0x8040); }', 1)
    fu = 'void disableFlushModeUnconditional() {} void enableFlushModeUnconditional() {}'
    assert s.count(fu) == 1
    s = s.replace(fu, 'void disableFlushModeUnconditional() { _mm_setcsr(_mm_getcsr() & ~0x8040); }'
                      ' void enableFlushModeUnconditional() { _mm_setcsr(_mm_getcsr() | 0x8040); }', 1)
    for old, new in (('objects[8192]', 'objects[0xF000]'), ('a<12288', 'a<0x10000')):
        assert s.count(old) == 1, old
        s = s.replace(old, new, 1)
    return s


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


def matrix(rnd):
    # (rows and place, 16 words, the w lanes 0 as the game keeps them)
    r = rotation(rnd) if rnd.random() < 0.85 else [[1.0 if i == j else 0.0 for j in range(3)] for i in range(3)]
    p = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 80), rnd.uniform(-3000, 3000)]
    out = []
    for row in r + [p]:
        out += [*map(fb, row), 0]
    return out


def hitch(rnd):
    h = rnd.choice(([0.0, 1.1, -2.6], [0.0, 1.2, 5.9], [rnd.uniform(-1, 1), rnd.uniform(0, 2), rnd.uniform(-8, 8)], [0.0, 0.0, 0.0]))
    return [*map(fb, h), rnd.choice((0, 0x3F800000))]


# (the level's pointer the routine hands AddPairConstraint its +8 from - filled by the running game, 0 in the image -,
# as an offset from BASE so the file cites no address; here a zeroed scratch block)
BASE = 0x828F << 16
LEVEL = BASE + 0xF710

IDENT = [0x3F800000, 0, 0, 0, 0, 0x3F800000, 0, 0, 0, 0, 0x3F800000, 0, 0, 0, 0, 0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    semi, init, lookup, add_pair, pre, active, wake = ('sub_' + addr(a.tool, f) for f in
                                                       ('F-15252', 'F-44049', 'F-18069', 'F-82928', 'F-27121', 'F-17857', 'F-10339'))
    rnd = random.Random(15252)
    cases = []
    for _ in range(600):
        cases.append(('pair', matrix(rnd) + matrix(rnd) + hitch(rnd) + hitch(rnd)))
    for _ in range(1000):
        ws = matrix(rnd) + matrix(rnd)
        for _k in range(2):
            ws += [*map(fb, (rnd.uniform(-3000, 3000), rnd.uniform(-20, 80), rnd.uniform(-3000, 3000))), rnd.choice((0, 0x3F800000))]
        for _k in range(2):
            ty = rnd.choice((12, 12, 2))
            part = rnd.choice((0, 0, 1))
            ident = rnd.random() < 0.7
            pm = []
            for _p in range(2):
                pm += IDENT if ident else matrix(rnd)
            ws += [ty, part] + pm
        ws.append(fb(rnd.choice((0.0, 6.0, 1.5, rnd.uniform(0, 10)))))
        cases.append(('init', ws))
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = []
    for kind, ws in cases:
        flat += [0 if kind == 'pair' else 1, len(ws)] + ws
    out4 = lambda at: f'load({at}),load({at}+4),load({at}+8),load({at}+12)'
    pf4 = '%08x %08x %08x %08x '
    stubs = {
        pre: '',
        lookup: 'ctx.r3.u32=0x2000;',
        add_pair: ('{float g=float(ctx.f1.f64);uint32_t u;memcpy(&u,&g,4);printf("' + pf4 * 2 + '%08x %x %x %x %x ",' +
                   f'{out4("ctx.r4.u32")},{out4("ctx.r5.u32")},u,ctx.r6.u32,ctx.r7.u32,ctx.r8.u32,ctx.r9.u32);}}' +
                   'ctx.r3.u32=0x4000+0x400*K;K++;'),
        active: 'ctx.r3.u32=1;',
        wake: '',
    }
    # (start-up constants, zero in the image, as their initialisers fill them - docs/reference/ai/raceline.md: the xyz mask
    # and (0, 0, 0, 1) just below it)
    seed = 'for(int k=0;k<4;k++){store(0x82860950+4*k,k<3?0xffffffff:0);store(0x82860930+4*k,k<3?0:0x3f800000);}'
    # (a car: its instance at +112 - matrix +128 .. +191 -, its tune +448 with byte +111 set, the hitch's chain)
    car = lambda base, w0, tune, t168, t204, h0: (
        f'for(int k=0;k<16;k++)store({base}+128+4*k,w[{w0}+k]);store({base}+448,{tune});store({tune}+108,0x00000001u);' +
        f'store({tune}+168,{t168});store({t168}+204,{t204});for(int k=0;k<4;k++)store({t204}+96+4*k,w[{h0}+k]);')
    pair = ('K=0;' + car(0x1000, 0, 0x3000, 0x3400, 0x3800, 32) + car(0x2000, 16, 0x3200, 0x3600, 0x3A00, 36) +
            'ctx.r13.u32=0x5000;store(0x5000,0x5100);' + f'store({LEVEL:#x},0x5200);' + 'ctx.r3.u32=0x1000;' + f'{semi}(ctx,nullptr);' +
            'printf("| ");for(int k=0;k<3;k++){uint32_t c=0x4000+0x400*k;printf("%08x %08x %08x %04x ",load(c+164),load(c+320)>>24,load(c+328),load(c+200)>>16);}')
    inst = lambda base, w0, arch, bound, parts, t0: (
        f'for(int k=0;k<16;k++)store({base}+16+4*k,w[{w0}+k]);store({base}+4,{arch});store({arch}+12,{bound});' +
        f'store({bound}+4,w[{t0}]<<24);store({bound}+132,{parts});for(int k=0;k<32;k++)store({parts}+4*k,w[{t0}+2+k]);')
    initc = (inst(0x6400, 0, 0x6800, 0x6900, 0x6A00, 40) + inst(0x6C00, 16, 0x7000, 0x7100, 0x7200, 74) +
             'for(int k=0;k<4;k++){store(0x7800+4*k,w[32+k]);store(0x7810+4*k,w[36+k]);}' +
             '{float l;memcpy(&l,&w[108],4);ctx.f1.f64=l;}ctx.r3.u32=0x6000;ctx.r4.u32=0x6400;ctx.r5.u32=0x7800;ctx.r6.u32=w[41];' +
             f'ctx.r7.u32=0x6C00;ctx.r8.u32=0x7810;ctx.r9.u32=w[75];{init}(ctx,nullptr);' +
             'printf("%x %x %x %x ' + pf4 * 4 + '%08x %08x",load(0x6000+24),load(0x6000+28),load(0x6000+188),load(0x6000+192),' +
             f'{out4(0x6000 + 80)},{out4(0x6000 + 96)},{out4(0x6000 + 112)},{out4(0x6000 + 128)},load(0x6000+324),load(0x6000+160));')
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'size_t at=0;for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));' + seed +
            'uint32_t kind=D[at],n=D[at+1];const uint32_t*w=&D[at+2];at+=2+n;' +
            'Context ctx{};ctx.r1.u32=0xF000;' +
            'if(kind==0){' + pair + '}else{' + initc + '}printf("' + NL + '");}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [semi, init], main_src, a.out_dir, 'semi_ropes', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s %s -> %s' % (kind, ' '.join('%x' % x for x in ws), o) for (kind, ws), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'semi_ropes_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
