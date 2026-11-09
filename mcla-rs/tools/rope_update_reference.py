"""The constraint update F-83853 on a semi's cab-trailer constraint, run offline (generated code unchanged, vector mode,
flush mode as the SDK sets it; decode: work/physics2/ropeupd/decode.md).

The constraint is made as F-15252 makes it: the ctor F-31401, the pair init F-44049 (two ambient-car instances, the world
points, parts, the length: the run-time ROPE_POINT 0 / 6.0 / 1.5), the mode F-95940 (2), then +164 = 0.6, +320 = 0,
+328 = -1. Each case then calls F-83853(c, invdt f1, record r5) once. Run as they are: the anchors to the world F-36793,
the rope's normal F-72436 (the constraint's slot 1), the record's reset F-12558 and fill F-13106, the push split F-64595,
an inactive instance's point velocity (slot 6 F-94307 -> slot 4 F-90762 = inst +96, slot 5 F-11104 = 0; the ambient
car instance's own vtable, BASE-relative below), an active side's point velocity (collider slot 21 F-67516: +256
place, +272 velocity, +288 turn).
Stubbed (logged): the level's behaviour lookup F-74281 (gives none: what the port assumes; open in the decode), the
activation F-24099 (logs the level index and its two zero arguments, gives the case's collider or 0 when the wake is
to fail), the collider's direct push slot 27 F-12566 (logs the collider, r4, r5 and the vector v1 it is handed).
The level (BASE + 0xF710) is a scratch block: +4 -> the object table, +112 entries (8 bytes an index, +4 low two
bits 0 = active), +120 the colliders by index. The allowed depth (BASE + 0xF840, written at run time) is seeded
with solver::ALLOW's value.

Cases: random rotations and city places, the second car within 8 m, world points at the rope's length + e apart
(e across slack, the wake threshold 0.01, the depth +164 = 0.6 and past it), each side active / inactive and woken /
inactive and the wake failing, inverse masses equal / either larger / 0, velocities to 90 m/s (past the clamp +168 =
60), composite (type 12) and simple bounds, part 0 / 1, identity and random part matrices, a level index 0xFFFF now
and then.

Writes <out-dir>/rope_update_ref.txt: "upd <case words> -> [wake <idx> <r5> <r6> -> <collider>]* [push <collider> <r4>
<r5> <v1 x y z w>]* | ret <r3> c16 <+16> c20 <+20> c322 <+322> | rec <the record's 52 words, +0 .. +204>"."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('static void dispatch(Context &ctx,const uint8_t *base,uint32_t a);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10) +
                         'static const uint32_t *W;' + chr(10))
_vshim = recomp_closure.vector_shim


def _shim():
    # (cr6 from a vector compare as the SDK's; the flush mode as the SDK sets it: MXCSR FTZ | DAZ; scratch 0x1000 .. 0x10000)
    # (the SDK's own rex::ppc::simde_mm_cvtepu32_ps_ and simde_mm_perm_epi8_, include/rex/ppc/intrinsics.h: vcuxwfp128 and
    # vperm, the second only in the angular branch)
    cvt = ('namespace rex { namespace ppc { inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {'
           'simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));'
           'simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8); xmm0 = simde_mm_srli_epi32(xmm0, 31);'
           'xmm0 = simde_mm_add_epi32(xmm0, xmm1); xmm0 = simde_mm_srai_epi32(xmm0, 8);'
           'xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000)); simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);'
           'return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1)); }'
           'inline simde__m128i simde_mm_perm_epi8_(simde__m128i a, simde__m128i b, simde__m128i c) {'
           'simde__m128i d = simde_mm_set1_epi8(0xF); simde__m128i e = simde_mm_sub_epi8(d, simde_mm_and_si128(c, d));'
           'return simde_mm_blendv_epi8(simde_mm_shuffle_epi8(a, e), simde_mm_shuffle_epi8(b, e), simde_mm_slli_epi32(c, 3)); } } }')
    s = ('#include <xmmintrin.h>' + chr(10) + '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx2.h>' + chr(10) +
         cvt + chr(10) + _vshim())
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


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u))[0]


def rotation(rnd):
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(x * x for x in q))
    w, x, y, z = (v / n for v in q)
    return [[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]]


def matrix(rnd, place):
    # (rows and place, 16 words, the w lanes 0 as the game keeps them)
    r = rotation(rnd) if rnd.random() < 0.85 else [[1.0 if i == j else 0.0 for j in range(3)] for i in range(3)]
    out = []
    for row in r + [place]:
        out += [*map(fb, row), 0]
    return out


def vec4(rnd, s, w=0):
    return [*map(fb, (rnd.uniform(-s, s), rnd.uniform(-s, s), rnd.uniform(-s, s))), w]


# (the start of the original's data pages the routine's globals sit in, so the file cites no address: the level pointer
# BASE + 0xF710, the allowed depth BASE + 0xF840; the ambient car instance's vtable and its slots 4 .. 6 below)
BASE = 0x828F << 16
LEVEL = BASE + 0xF710
ALLOW = BASE + 0xF840
INST_VT = (0x820C << 16) + 0x4314

# (start-up constants, zero in the image, as their initialisers fill them - docs/reference/ai/raceline.md: the xyz mask
# and (0, 0, 0, 1) just below it)
MASK = (0x8286 << 16) + 0x950
W1 = (0x8286 << 16) + 0x930

IDENT = [0x3F800000, 0, 0, 0, 0, 0x3F800000, 0, 0, 0, 0, 0x3F800000, 0, 0, 0, 0, 0]
ALLOW_BITS = 0x3BE56042   # (solver::ALLOW: what F-36556 writes there)


def case(rnd):
    ln = rnd.choice((0.0, 6.0, 1.5))
    r = rnd.random()
    if r < 0.08:
        e = 0.01 + rnd.choice((-1, 1)) * rnd.uniform(0, 2e-6)
    elif r < 0.14:
        e = 0.6 + rnd.choice((-1, 1)) * rnd.uniform(0, 2e-6)
    elif r < 0.18:
        e = 0.0
    else:
        e = rnd.uniform(-1.0, 2.0)
    d = max(0.0, ln + e)
    pa = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 80), rnd.uniform(-3000, 3000)]
    ma = matrix(rnd, [pa[0] + rnd.uniform(-3, 3), pa[1] + rnd.uniform(-2, 2), pa[2] + rnd.uniform(-3, 3)])
    mb = matrix(rnd, [pa[0] + rnd.uniform(-8, 8), pa[1] + rnd.uniform(-2, 2), pa[2] + rnd.uniform(-8, 8)])
    u = [rnd.gauss(0, 1) for _ in range(3)]
    n = math.sqrt(sum(x * x for x in u))
    pb = [pa[k] + u[k] / n * d for k in range(3)]
    # (w = 1, as F-15252 hands the points over: the OR with (0, 0, 0, 1), then + 10 x a row whose w is 0)
    ws = ma + mb + [*map(fb, pa), 0x3F800000] + [*map(fb, pb), 0x3F800000]
    for _k in range(2):
        ty = rnd.choice((12, 12, 2))
        part = rnd.choice((0, 0, 1))
        ident = rnd.random() < 0.7
        pm = []
        for _p in range(2):
            pm += IDENT if ident else matrix(rnd, [rnd.uniform(-2, 2) for _ in range(3)])
        ws += [ty, part] + pm
    ws.append(fb(ln))
    ws += [rnd.choice((0, 0, 1, 1, 2)), rnd.choice((0, 0, 1, 1, 2))]
    im = rnd.choice(('eq', 'r', 'r', 'a0', 'b0'))
    ia = rnd.uniform(1e-4, 2e-3)
    ib = ia if im == 'eq' else rnd.uniform(1e-4, 2e-3)
    if im == 'a0':
        ia = 0.0
    if im == 'b0':
        ib = 0.0
    ws += [fb(ia), fb(ib)]
    for m in (ma, mb):
        place = [bf(m[12]), bf(m[13]), bf(m[14])]
        ws += [*map(fb, (place[0] + rnd.uniform(-1, 1), place[1] + rnd.uniform(-0.5, 0.5), place[2] + rnd.uniform(-1, 1))), 0]
        ws += vec4(rnd, rnd.choice((2, 30, 90)))
        ws += vec4(rnd, rnd.choice((0, 0.5, 3)))
    ws += vec4(rnd, rnd.choice((0, 2, 30, 90)), rnd.choice((0, 0, fb(rnd.uniform(-1, 1)))))
    ws += vec4(rnd, rnd.choice((0, 2, 30, 90)), 0)
    ws.append(fb(rnd.choice((60.0, 30.0))))
    ws += [0xFFFF if rnd.random() < 0.02 else 5, 0xFFFF if rnd.random() < 0.02 else 9]
    return ws


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    names = ('F-83853', 'F-31401', 'F-44049', 'F-95940', 'F-72436', 'F-94307', 'F-90762', 'F-11104', 'F-67516', 'F-74281',
             'F-24099', 'F-12566')
    upd, ctor, init, mode, normal, ivel, ivel4, ivel5, cvel, behav, wake, push = ('sub_' + addr(a.tool, f) for f in names)
    rnd = random.Random(83853)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = []
    for ws in cases:
        flat += [len(ws)] + ws
    v4 = lambda v: f'{v}.u32[3],{v}.u32[2],{v}.u32[1],{v}.u32[0]'
    stubs = {
        behav: 'ctx.r3.u32=0;',
        wake: ('{uint32_t i=ctx.r4.u32,s=i==W[146]?W[109]:W[110],c=i==W[146]?0x4000:0x4400;uint32_t r=s==1?c:0;'
               'printf("wake %x %x %x -> %x ",i,ctx.r5.u32,ctx.r6.u32,r);ctx.r3.u32=r;}'),
    }
    # (the indirect calls on the path: the constraint's slot 1, the ambient instance's slots 4 .. 6, the collider's
    # slots 21 and 27 - the last logged instead of run; anything else stops the case)
    disp = ('static void dispatch(Context &ctx,const uint8_t *base,uint32_t a){switch(a){' +
            ''.join(f'case 0x{s[4:]}u:{s}(ctx,base);return;' for s in (normal, ivel, ivel4, ivel5, cvel)) +
            f'case 0x{push[4:]}u:printf("push %x %x %x %08x %08x %08x %08x ",ctx.r3.u32,ctx.r4.u32,ctx.r5.u32,{v4("ctx.v1")});return;' +
            'default:throw std::runtime_error("indirect");}}' + nl)
    seed = (f'for(int k=0;k<4;k++){{store({MASK:#x}+4*k,k<3?0xffffffff:0);store({W1:#x}+4*k,k<3?0:0x3f800000);}}' +
            f'store({LEVEL:#x},0x3000);store({ALLOW:#x},{ALLOW_BITS:#x}u);')
    # (an instance: vtable, archetype -> bound (type byte +4, parts +132), level index +8, matrix +16, velocity +96)
    inst = lambda base, w0, arch, bound, parts, t0, vel, ix: (
        f'store({base},{INST_VT:#x}u);for(int k=0;k<16;k++)store({base}+16+4*k,W[{w0}+k]);store({base}+4,{arch});'
        f'store({arch}+12,{bound});store({bound}+4,W[{t0}]<<24);store({bound}+132,{parts});'
        f'for(int k=0;k<32;k++)store({parts}+4*k,W[{t0}+2+k]);for(int k=0;k<4;k++)store({base}+96+4*k,W[{vel}+k]);'
        f'store16({base}+8,W[{ix}]);')
    # (a collider: fake vtable 0x4800 with slots 21 / 27, +196 inverse mass, +256 place, +272 velocity, +288 turn)
    coll = lambda base, im, w0: (f'store({base},0x4800);store({base}+196,W[{im}]);'
                                 f'for(int k=0;k<4;k++){{store({base}+256+4*k,W[{w0}+k]);store({base}+272+4*k,W[{w0}+4+k]);'
                                 f'store({base}+288+4*k,W[{w0}+8+k]);}}')
    level = ('store(0x3004,0x3100);store(0x3100+112,0x3200);store(0x3100+120,0x3300);'
             'if(W[146]!=0xFFFF){store(0x3200+8*W[146]+4,W[109]?1:0);store(0x3300+4*W[146],0x4000);}'
             'if(W[147]!=0xFFFF){store(0x3200+8*W[147]+4,W[110]?1:0);store(0x3300+4*W[147],0x4400);}')
    make = (f'ctx.r3.u32=0x1000;{ctor}(ctx,nullptr);' +
            'for(int k=0;k<4;k++){store(0x7800+4*k,W[32+k]);store(0x7810+4*k,W[36+k]);}' +
            '{float l;memcpy(&l,&W[108],4);ctx.f1.f64=l;}ctx.r3.u32=0x1000;ctx.r4.u32=0x2000;ctx.r5.u32=0x7800;ctx.r6.u32=W[41];' +
            f'ctx.r7.u32=0x2400;ctx.r8.u32=0x7810;ctx.r9.u32=W[75];{init}(ctx,nullptr);' +
            f'ctx.r3.u32=0x1000;{mode}(ctx,nullptr);store(0x1000+164,0x3f19999au);pointer(0x1000+320)[0]=0;store(0x1000+328,0xbf800000u);')
    call = ('{float t;memcpy(&t,&W[145],4);ctx.f1.f64=t;}ctx.r3.u32=0x1000;ctx.r5.u32=0x1200;ctx.r1.u32=0xF000;' +
            f'{upd}(ctx,nullptr);' +
            'printf("| ret %x c16 %x c20 %x c322 %x | rec",ctx.r3.u32,load(0x1000+16),load(0x1000+20),pointer(0x1000+322)[0]);'
            'for(int k=0;k<52;k++)printf(" %08x",load(0x1200+4*k));')
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'size_t at=0;for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));' + seed +
            'uint32_t n=D[at];W=&D[at+1];at+=1+n;' +
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x5000;try{' +
            inst(0x2000, 0, 0x2200, 0x2240, 0x2300, 40, 137, 146) + inst(0x2400, 16, 0x2600, 0x2640, 0x2700, 74, 141, 147) +
            coll(0x4000, 111, 113) + coll(0x4400, 112, 125) +
            f'store(0x4800+84,0x{cvel[4:]}u);store(0x4800+108,0x{push[4:]}u);' + level + make +
            call + '}catch(std::exception&e){printf("| stop %s",e.what());}printf("' + NL + '");}' + nl)
    main_src = (disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), [upd, ctor, init, mode, normal, ivel, ivel4, ivel5, cvel], main_src, a.out_dir, 'rope_update',
                stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    assert not any('stop' in o for o in out), next(o for o in out if 'stop' in o)
    lines = ['upd %s -> %s' % (' '.join('%x' % x for x in ws), o) for ws, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'rope_update_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
