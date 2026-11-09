"""The ambient manager's 'is this point seen' test (F-95989) run offline: the generated code whole and unchanged in
vector mode, with the C library's acos (F-68873) in the closure, on flat guest memory. Stubbed: the occluder query
(F-87100), answering the case's bits and logging its arguments. The manager S at 0x1000: +4 / +8 the spawn inner /
outer radius, +72 the wide radius (the flag's), +108 / +112 the field of view's low / high mph, +116 / +120 its degrees
at them, +192 the focus (x, y, z), +224 / +232 the view's x / z, +968 the speed in mph, byte +2121 the occluder test on;
r4 the point (0x1900), f1 the height added to it for the occluder, r6 the flag.
Writes <out-dir>/seen_ref.txt: the case's words, then '->', the answer and the occluder call ('-' none, else the
point's x y z, r4, r7, f1 and the global passed, all hex)."""

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
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic const uint32_t *K;\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

GLOBAL = 0x8288BB24


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
    seen, occ = (addr(a.tool, f) for f in ('F-95989', 'F-87100'))
    rnd = random.Random(95989)
    nan = float('nan')
    cases = []
    for _ in range(6000):
        inner, outer = rnd.choice(((50.0, 250.0), (rnd.uniform(0, 80), rnd.uniform(80, 300)), (120.0, 400.0)))
        wide = rnd.choice((250.0, 400.0, rnd.uniform(0, 300)))
        lo, hi = rnd.choice(((25.0, 45.0), (rnd.uniform(0, 50), rnd.uniform(0, 80)), (30.0, 30.0)))
        dl, dh = rnd.choice(((89.9, 45.0), (rnd.uniform(0, 180), rnd.uniform(0, 180))))
        fx, fy, fz = rnd.uniform(-3000, 3000), rnd.uniform(-50, 200), rnd.uniform(-3000, 3000)
        yaw = rnd.uniform(-math.pi, math.pi)
        k = rnd.choice((1.0, 1.0, 1.0, rnd.uniform(0.5, 1.5)))
        vx, vz = math.sin(yaw) * k, math.cos(yaw) * k
        mph = rnd.choice((rnd.uniform(0, 120), lo, hi, rnd.uniform(-10, 10), nan, 0.0))
        r = rnd.choice((rnd.uniform(0, 450), inner, outer, wide, 0.0, rnd.uniform(0, 1e-3)))
        ang = rnd.choice((rnd.uniform(-math.pi, math.pi), yaw + rnd.uniform(-1.6, 1.6), yaw + math.pi + rnd.uniform(-0.8, 0.8)))
        px, py, pz = fx + r * math.sin(ang), fy + rnd.uniform(-10, 10), fz + r * math.cos(ang)
        if rnd.random() < 0.03:
            px, pz = fx, fz
        h = rnd.choice((1.5, 1.75, rnd.uniform(0, 3)))
        flag = rnd.choice((0, 0, 1, 0x100, 0x101))
        on = rnd.choice((0, 1, 1, 2))
        bits = rnd.choice((0, 0x04000000, 0x10000000, 0x14000000, 0x08000000, 0x7FFFFFFF ^ 0x14000000, rnd.getrandbits(32)))
        cases.append([fb(inner), fb(outer), fb(wide), fb(lo), fb(hi), fb(dl), fb(dh), fb(fx), fb(fy), fb(fz),
                      fb(vx), fb(vz), fb(mph), fb(px), fb(py), fb(pz), fb(h), flag, on, bits])
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    # the occluder query: log the point (r5) and the arguments at 0x2FC0, answer the case's bits
    stub = ('store(0x2FC0,1);for(int j=0;j<3;j++)store(0x2FC4+4*j,load(ctx.r5.u32+4*j));store(0x2FD0,ctx.r4.u32);'
            'store(0x2FD4,ctx.r7.u32);float f=float(ctx.f1.f64);uint32_t u;memcpy(&u,&f,4);store(0x2FD8,u);'
            'store(0x2FDC,ctx.r3.u32);ctx.r3.u32=K[19];')
    setup = ('memset(objects,0,sizeof(objects));K=c;'
             'for(int k=0;k<4;k++){put(0x827D8450+4*k,0x3f800000);put(0x827D8460+4*k,0x3f000000);put(0x828609E0+4*k,0x7f800000);put(0x828608C0+4*k,0);}'
             'put(0x%x,0x5A5A1230);' % GLOBAL +
             'store(0x1004,c[0]);store(0x1008,c[1]);store(0x1048,c[2]);store(0x106C,c[3]);store(0x1070,c[4]);store(0x1074,c[5]);store(0x1078,c[6]);'
             'store(0x10C0,c[7]);store(0x10C4,c[8]);store(0x10C8,c[9]);store(0x10CC,0x3f800000);store(0x10E0,c[10]);store(0x10E8,c[11]);'
             'store(0x13C8,c[12]);objects[0x1000+2121-0x1000]=uint8_t(c[18]);'
             'store(0x1900,c[13]);store(0x1904,c[14]);store(0x1908,c[15]);store(0x190C,0x3f800000);')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1900;ctx.r6.u32=c[17];'
            'float h;memcpy(&h,&c[16],4);ctx.f1.f64=h;'
            'sub_%s(ctx,nullptr);uint32_t r=ctx.r3.u32;' % seen +
            'if(load(0x2FC0))printf("%%u %%08x %%08x %%08x %%x %%x %%08x %%08x%s",r,load(0x2FC4),load(0x2FC8),load(0x2FCC),load(0x2FD0),load(0x2FD4),load(0x2FD8),load(0x2FDC));' % NL +
            'else printf("%%u -%s",r);}' % NL + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + seen], main_src, a.out_dir, 'seen', stubs={'sub_' + occ: stub}, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'seen_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] == '1' for o in out), 'seen,', sum(o.split()[1] != '-' for o in out), 'occluder calls')


if __name__ == '__main__':
    main()
