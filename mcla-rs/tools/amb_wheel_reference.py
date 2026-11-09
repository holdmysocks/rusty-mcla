"""An active ambient car's wheel (F-19974, slot 14's per-wheel step) run offline: the generated code whole and unchanged in
vector mode, with the quarter weight F-60791 (its archetype's mass call answered with the case's mass) and the register
save / restore helpers in the closure, on flat guest memory (grown to 0x8000). Stubbed: the instance's matrix getter F-33765
(answers the case's matrix) and the forward getter F-50349 (writes the negated row 2, as it reads the same matrix). The body's
virtual calls are caught: +84 the point velocity (answers the case's velocity), +128 the force, +132 the torque, +108 the push,
+124 the force at a point (their vector arguments recorded).
Memory: the wheel record W (r3 = 0x1000: +0 the body, +4 the owner, +8 d, +12 s, +16 raw, +20 slide), the owner (+448 the
type: +104 the archetype; +204 grip, +216 travel, +228 stretch, +240 cog, +244 k, +248 c, +252 kt, +256 ct), the body (+272
its velocity), the hit (r6: +0 hit, +16 point, +32 normal), r7 the probe's middle, f1 dt, r5 1 (apply), r8 the front
pair, r9 0.
Writes <out-dir>/amb_wheel_ref.txt: the case's words, '->', d s raw slide, then each call 'name x y z' (the tyre's point
after it), all hex."""

import argparse
import math
import random
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
    return s


_shim = recomp_closure.shim
recomp_closure.shim = lambda scratch_end=0x3000: _shim(0x8000)
recomp_closure.vector_shim = vector_shim
NL = chr(92) + 'n'
recomp_closure.EXTRA += ('static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10) +
                         'static const uint32_t *K;' + chr(10) +
                         'static void ind(Context&ctx,uint32_t a);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) ind(ctx,a)' + chr(10))
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(v):
    n = math.sqrt(sum(c * c for c in v)) or 1.0
    return [c / n for c in v]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def matrix(rnd):
    """rows x, up, z (a rotation: yaw any, pitch / roll mostly small, sometimes tumbled), the place"""
    if rnd.random() < 0.7:
        tilt = rnd.uniform(0, 0.3)
    else:
        tilt = rnd.uniform(0, math.pi)
    u = unit([math.sin(tilt) * math.cos(a := rnd.uniform(-math.pi, math.pi)), math.cos(tilt), math.sin(tilt) * math.sin(a)])
    f = unit(cross(u, unit([rnd.uniform(-1, 1), rnd.uniform(-1, 1), rnd.uniform(-1, 1)])))
    x = unit(cross(u, f))
    z = cross(x, u)
    p = [rnd.uniform(-3000, 3000), rnd.uniform(-10, 300), rnd.uniform(-3000, 3000)]
    return x, u, z, p


def case(rnd):
    x, u, z, p = matrix(rnd)
    wx, wz = rnd.choice((1.3, -1.3, 0.8, -0.8)), rnd.choice((1.5, -1.5, 2.2, -1.9))
    mid = [p[k] + x[k] * wx + z[k] * wz for k in range(3)]
    travel = rnd.choice((0.2, 0.15, 0.25, rnd.uniform(0.05, 0.4)))
    d_old = rnd.choice((0.0, travel, -travel, rnd.uniform(-travel, travel), rnd.uniform(-travel, travel)))
    s_old = rnd.choice((0.0, rnd.uniform(0, 0.2), rnd.uniform(-0.2, 0.2)))
    raw_old = rnd.uniform(0, 0.3)
    slide_old = rnd.choice((0, 1))
    hit = int(rnd.random() < 0.85)
    depth = rnd.choice((rnd.uniform(-0.5, 0.5), rnd.uniform(0.2, 0.5), rnd.uniform(-0.05, 0.05)))
    n = unit([rnd.uniform(-0.4, 0.4), 1.0, rnd.uniform(-0.4, 0.4)]) if rnd.random() < 0.8 else unit([rnd.uniform(-1, 1), rnd.uniform(0.7, 1), rnd.uniform(-1, 1)])
    contact = [mid[k] + u[k] * depth + rnd.uniform(-0.05, 0.05) for k in range(3)]
    v = rnd.choice(([rnd.uniform(-30, 30), rnd.uniform(-15, 15), rnd.uniform(-30, 30)], [rnd.uniform(-0.6, 0.6) for _ in range(3)],
                    [0.0, rnd.uniform(-13, 0), 0.0]))
    vp = [c + rnd.uniform(-2, 2) for c in v] if rnd.random() < 0.6 else list(v)
    dt = rnd.choice((1 / 60, 1 / 30, rnd.uniform(0.005, 0.05)))
    front = rnd.choice((0, 1))
    mass = rnd.choice((600.0, 1500.0, rnd.uniform(400, 3000)))
    wq = mass * 9.8 / 4
    k = rnd.uniform(1.0, 3.0) * wq / travel
    c = 2 * rnd.uniform(0.1, 0.6) * math.sqrt(wq * k)
    stretch = rnd.choice((0.05, rnd.uniform(0.01, 0.2)))
    kt = 2 * wq * rnd.uniform(0.8, 1.5) / stretch
    ct = 2 * rnd.uniform(0.05, 0.5) * math.sqrt(wq * kt)
    grip = rnd.uniform(0.3, 1.0)
    cog = rnd.uniform(-0.3, 0.3)
    words = [*x, *u, *z, *p, d_old, s_old, raw_old, *contact, *n, *mid, *v, *vp, dt, grip, travel, stretch, cog, k, c, kt, ct, mass]
    return [fb(w) for w in words] + [slide_old, hit, front]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    wheel, mat, fwd = addr(a.tool, 'F-19974'), addr(a.tool, 'F-33765'), addr(a.tool, 'F-50349')
    rnd = random.Random(19974)
    cases = [case(rnd) for _ in range(4000)]
    W = len(cases[0])
    nl = chr(10)
    rows_c = nl.join('{%s},' % ','.join('0x%08x' % w for w in c) for c in cases)
    # case words: 0..11 matrix rows x u z + place, 12 d, 13 s, 14 raw, 15..17 contact, 18..20 normal, 21..23 mid, 24..26 v,
    # 27..29 vp, 30 dt, 31 grip, 32 travel, 33 stretch, 34 cog, 35 k, 36 c, 37 kt, 38 ct, 39 mass, 40 slide, 41 hit, 42 front
    ind = ('static void vec(Context&ctx,const char*n,VReg&v){printf(" %s %08x %08x %08x",n,v.u32[3],v.u32[2],v.u32[1]);}' + nl +
           'static void ind(Context&ctx,uint32_t a){switch(a){'
           'case 0x0F000084:{uint32_t o=ctx.r4.u32;store(o,K[27]);store(o+4,K[28]);store(o+8,K[29]);store(o+12,0);break;}'
           'case 0x0F000128:vec(ctx,"f",ctx.v1);break;'
           'case 0x0F000132:vec(ctx,"t",ctx.v1);break;'
           'case 0x0F000108:vec(ctx,"p",ctx.v1);break;'
           'case 0x0F000124:vec(ctx,"y",ctx.v1);vec(ctx,"at",ctx.v2);break;'
           'case 0x0F000028:{uint32_t b=K[39];float m;memcpy(&m,&b,4);ctx.f1.f64=m;break;}'
           'default:throw std::runtime_error("indirect");}}' + nl)
    stubs = {
        'sub_' + mat: 'ctx.r3.u32=0x1600;',
        'sub_' + fwd: 'uint32_t o=ctx.r3.u32;for(int j=0;j<3;j++)store(o+4*j,K[6+j]^0x80000000u);store(o+12,0);',
    }
    setup = ('memset(objects,0,sizeof(objects));K=c;'
             'for(int k=0;k<4;k++){put(0x828609E0+4*k,0x7f800000);put(0x828608C0+4*k,0);put(0x827D8450+4*k,0x3f800000);put(0x827D8460+4*k,0x3f000000);}'
             # W, body 0x1100 (vtable 0x1A00), owner 0x1200 (type 0x1400, archetype 0x1300 with vtable 0x1B00)
             'store(0x1000,0x1100);store(0x1004,0x1200);store(0x1008,c[12]);store(0x100C,c[13]);store(0x1010,c[14]);objects[0x14]=c[40];'
             'store(0x1100,0x1A00);store(0x1200+448,0x1400);store(0x1400+104,0x1300);store(0x1300,0x1B00);store(0x1B00+28,0x0F000028);'
             'for(int o:{84,128,132,108,124})store(0x1A00+o,0x0F000000+(o/100)*0x100+((o/10)%10)*0x10+o%10);'
             'for(int j=0;j<3;j++)store(0x1100+272+4*j,c[24+j]);'
             'store(0x1400+204,c[31]);store(0x1400+216,c[32]);store(0x1400+228,c[33]);store(0x1400+240,c[34]);'
             'store(0x1400+244,c[35]);store(0x1400+248,c[36]);store(0x1400+252,c[37]);store(0x1400+256,c[38]);'
             'for(int r=0;r<4;r++){for(int j=0;j<3;j++)store(0x1600+16*r+4*j,c[3*r+j]);store(0x1600+16*r+12,r==3?0x3f800000:0);}'
             'store(0x1800,c[41]?0x5000:0);for(int j=0;j<3;j++){store(0x1800+16+4*j,c[15+j]);store(0x1800+32+4*j,c[18+j]);store(0x1900+4*j,c[21+j]);}')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows_c + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x1000;ctx.r5.u32=1;ctx.r6.u32=0x1800;ctx.r7.u32=0x1900;ctx.r8.u32=c[42];ctx.r9.u32=0;'
            '{uint32_t b=c[30];float dt;memcpy(&dt,&b,4);ctx.f1.f64=dt;}'
            'sub_%s(ctx,nullptr);' % wheel +
            ('printf(" | %%08x %%08x %%08x %%d%s",load(0x1008),load(0x100C),load(0x1010),objects[0x14]);}' % NL) + nl)
    main_src = (ind + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + wheel], main_src, a.out_dir, 'amb_wheel', stubs=stubs, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'amb_wheel_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(' p ' in o for o in out), 'pushes,', sum(' y ' in o for o in out), 'tyre forces')


if __name__ == '__main__':
    main()
