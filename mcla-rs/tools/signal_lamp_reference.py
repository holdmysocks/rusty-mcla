"""Run the original's signal head lamp F-39998 offline, as the generated code has it, and record what it hands on, for
the port's test (crates/game/src/testdata/signal_lamp_reference.txt).

Inputs: r4 the head's matrix (rows x, y, z, position at +16 / +32 / +48 / +64), r5 the lamp's locator in the head
(w ignored: the code masks xyz and ors in 1), r6 the reflection flag; the camera at *(0x82839704) + 112, the signal glow
strength 0x8287E254, size 0x8287E240 and the reflection sizes 0x8287E2D4 / D8 set per case. The start-up constants the
image holds as zero are set as the running game has them (0x82860950 the xyz mask, 0x82860930 (0, 0, 0, 1),
0x828609E0 +inf, 0x828608C0 0, 0x827D8450 1). The world-matrix set F-? (sub_8217C088) is stubbed empty; the glow add
(sub_822FC250) and the reflection add (sub_822FB508) are stubbed to print their place (v1), f1, f2, r4's two floats and r7.

  CASE <rows 16 words> <locator xyz> <refl> <cam xyz> <strength size rsx rsy>
  GLOW <P xyz> <f1> <f2> <size x y> <r7>  |  REFL <P xyz> <f1> <rsize x y>  |  (nothing: under 0.001)

python signal_lamp_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if n > 1e-3:
            return [x / n for x in v]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def cases(rnd):
    out = []
    for k in range(4000):
        yaw = rnd.uniform(-math.pi, math.pi)
        if k % 4 == 0:
            z = unit(rnd)
            x = cross([0.0, 1.0, 0.0], z)
            n = math.sqrt(sum(v * v for v in x)) or 1.0
            x = [v / n for v in x]
            y = cross(z, x)
        else:
            x, y, z = [math.cos(yaw), 0.0, -math.sin(yaw)], [0.0, 1.0, 0.0], [math.sin(yaw), 0.0, math.cos(yaw)]
        pos = [rnd.uniform(-3000, 3000), rnd.uniform(-5, 60), rnd.uniform(-2500, 2500)]
        rows = x + [0.0] + y + [0.0] + z + [0.0] + pos + [1.0]
        loc = [rnd.uniform(-1, 1), rnd.uniform(0, 6), rnd.uniform(-0.5, 0.5)]
        refl = k % 3 == 0
        if k % 5 == 0:
            cam = [pos[i] + z[i] * rnd.uniform(-60, 60) + rnd.uniform(-3, 3) for i in range(3)]
        else:
            cam = [pos[i] + rnd.uniform(-200, 200) for i in range(3)]
        if k % 41 == 0:  # the camera on the glow's place
            cam = [pos[i] + x[i] * loc[0] + y[i] * loc[1] + z[i] * (loc[2] - 0.3) for i in range(3)]
        st = rnd.choice((1.0, 0.5, 2.0, rnd.uniform(0, 4), 0.0005))
        sz, rx, ry = rnd.uniform(0.05, 2), rnd.uniform(0, 100), rnd.uniform(0, 100)
        out.append([fbits(v) for v in rows + loc] + [int(refl)] + [fbits(v) for v in cam + [st, sz, rx, ry]])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-39998')
    cs = cases(random.Random(39))
    nl, NL = chr(10), chr(92) + 'n'
    pv = 'printf(" %08x %08x %08x",ctx.v1.u32[3],ctx.v1.u32[2],ctx.v1.u32[1]);float g1=float(ctx.f1.f64);uint32_t b1;memcpy(&b1,&g1,4);printf(" %08x",b1);'
    stubs = {'sub_8217C088': '',
             'sub_822FC250': 'printf("GLOW");' + pv + 'float g2=float(ctx.f2.f64);uint32_t b2;memcpy(&b2,&g2,4);printf(" %08x %08x %08x %d' + NL + '",b2,load(ctx.r4.u32),load(ctx.r4.u32+4),ctx.r7.s32);',
             'sub_822FB508': 'printf("REFL");' + pv + 'printf(" %08x %08x' + NL + '",load(ctx.r4.u32),load(ctx.r4.u32+4));'}
    body = ('static const uint32_t C[][27]={' + ','.join('{' + ','.join('0x%08xu' % v for v in c) + '}' for c in cs) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'for(int i=0;i<3;i++)store(0x82860950+4*i,0xFFFFFFFFu);store(0x8286095C,0);for(int i=0;i<3;i++)store(0x82860930+4*i,0);store(0x8286093C,0x3F800000u);' +
            'for(int k=0;k<4;k++){store(0x828609E0+4*k,0x7F800000u);store(0x828608C0+4*k,0);store(0x827D8450+4*k,0x3F800000u);}' +
            # the matrix object at 0x1000 (rows at +16), the locator at 0x1100, the camera object at 0x1200 (+112), the glow object at 0x1300
            'for(int i=0;i<16;i++)store(0x1010+4*i,c[i]);for(int i=0;i<3;i++)store(0x1100+4*i,c[16+i]);store(0x110C,0x7FC00000u);' +
            'store(0x82839704,0x1200);for(int i=0;i<3;i++)store(0x1200+112+4*i,c[20+i]);store(0x1200+124,0x3F800000u);' +
            'store(0x8287E254,c[23]);store(0x8287E240,c[24]);store(0x8287E2D4,c[25]);store(0x8287E2D8,c[26]);' +
            'printf("CASE");for(int i=0;i<27;i++)printf(" %08x",c[i]);printf("' + NL + '");' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1300;ctx.r4.u32=0x1000;ctx.r5.u32=0x1100;ctx.r6.u32=c[19];' +
            ('sub_%s(ctx,nullptr);}}' % fn) + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'signal_lamp', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases', out.count('GLOW'), 'glows', out.count('REFL'), 'refls')


if __name__ == '__main__':
    main()
