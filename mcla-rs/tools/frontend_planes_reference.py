"""The ambient manager's front-end frustum (F-91754) run offline: the generated code whole and unchanged in vector mode,
with the C library's atan / sin / cos (F-79102, F-63589, F-72580) and the register save / restore helpers in the closure, on
flat guest memory (grown to 0x8000). r3 = the front-end points object S (+8256 / +8260 near / far, +8264 / +8268 the
widenings; it writes +8208 / +8212 the two atans, +8224 the place, +8240 the negated z row), r4 = the viewport (+64 / +80 /
+96 / +112 the matrix rows x, y, z, place; +712 / +716 the tangents), r5 = the six planes out (16 bytes each). The vector
constants 0x82860930 (0, 0, 0, 1) and 0x82860950 (the xyz mask) set as the game's start-up does.
Writes <out-dir>/frontend_planes_ref.txt: the case's words (the rows' x y z, tan +712, +716, the widenings, near, far), then
'->', the two atans (+8208, +8212) and the 24 plane words, all hex."""

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


# (the object S reaches +8268: flat memory 0x1000 - 0x8000)
_shim = recomp_closure.shim
recomp_closure.shim = lambda scratch_end=0x3000: _shim(0x8000)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic const uint32_t *K;\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def rows(rnd):
    """the matrix rows: a camera's (yaw, pitch, the port's construction) or random ones"""
    if rnd.random() < 0.8:
        yaw, pitch = rnd.uniform(-math.pi, math.pi), rnd.uniform(-1.2, 1.2)
        f = (math.sin(yaw) * math.cos(pitch), math.sin(pitch), -math.cos(yaw) * math.cos(pitch))
        z = tuple(-c for c in f)
        x = (z[2], 0.0, -z[0])
        n = math.hypot(x[0], x[2]) or 1.0
        x = (x[0] / n, 0.0, x[2] / n)
        y = (z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0])
    else:
        x, y, z = ([rnd.uniform(-1.5, 1.5) for _ in range(3)] for _ in range(3))
    eye = rnd.choice(([rnd.uniform(-3000, 3000), rnd.uniform(-50, 300), rnd.uniform(-3000, 3000)], [0.0, 0.0, 0.0]))
    return [*x, *y, *z, *eye]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    planes = addr(a.tool, 'F-91754')
    rnd = random.Random(91754)
    cases = []
    for _ in range(3000):
        r = rows(rnd)
        t = [rnd.choice((rnd.uniform(0.05, 1.0), rnd.uniform(1.0, 3.0), 0.0, rnd.uniform(-1, 1), 0.2679491924311227, 1.0, rnd.uniform(1e3, 1e6)))
             for _ in range(2)]
        w = [rnd.choice((0.0, 0.0, rnd.uniform(-0.1, 0.2))) for _ in range(2)]
        near, far = rnd.choice((0.1, 5.0, rnd.uniform(0.01, 10))), rnd.choice((226.0, 10000.0, rnd.uniform(50, 1000)))
        cases.append([fb(v) for v in r + t + w + [near, far]])
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows_c = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    setup = ('memset(objects,0,sizeof(objects));K=c;'
             'for(int k=0;k<4;k++){put(0x82860930+4*k,k<3?0:0x3f800000);put(0x82860950+4*k,k<3?0xFFFFFFFFu:0);}'
             'for(int r=0;r<4;r++){for(int j=0;j<3;j++)store(0x4040+16*r+4*j,c[3*r+j]);store(0x4040+16*r+12,r==3?0x3f800000:0);}'
             'store(0x4000+712,c[12]);store(0x4000+716,c[13]);store(0x1000+8264,c[14]);store(0x1000+8268,c[15]);'
             'store(0x1000+8256,c[16]);store(0x1000+8260,c[17]);')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows_c + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x4000;ctx.r5.u32=0x5000;'
            'sub_%s(ctx,nullptr);printf("%%08x %%08x",load(0x1000+8208),load(0x1000+8212));' % planes +
            ('for(int i=0;i<24;i++)printf(" %%08x",load(0x5000+4*i));printf("%s");}' % NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + planes], main_src, a.out_dir, 'frontend_planes', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'frontend_planes_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
