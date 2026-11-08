"""Run the original's main-view plane test of the cascade casters F-37078 offline, as the generated code has it, and
record its answer for the port's test (crates/game/src/render/testdata/casc_planes_reference.txt).

Inputs: r4 the view (its near, left and right planes as four floats at +768 / +784 / +800), r5 the sphere (centre, radius
at +12), r6 the light's direction (three floats). Answer: r3 (a bit per plane the sphere lies wholly outside of while the
light runs outwards through it).

  CASE <plane0 xyzw> <plane1 xyzw> <plane2 xyzw> <sphere xyzr> <dir xyz> -> <r3>

python casc_planes_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file>
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


def cases(rnd):
    out = []
    for k in range(6000):
        eye = [rnd.uniform(-3000, 3000), rnd.uniform(-10, 120), rnd.uniform(-2500, 2500)]
        pl = []
        for _ in range(3):
            n = unit(rnd)
            if k % 7 == 0:
                n = [0.0, 0.0, 1.0] if rnd.random() < 0.5 else [1.0, 0.0, 0.0]
            pl.append(n + [-sum(a * b for a, b in zip(n, eye)) + rnd.uniform(-1, 1)])
        c = [e + rnd.uniform(-300, 300) for e in eye]
        r = rnd.choice((0.0, rnd.uniform(0, 5), rnd.uniform(0, 100)))
        if k % 5 == 0:  # on a plane's edge
            p = rnd.choice(pl)
            dd = sum(a * b for a, b in zip(p, c)) + p[3]
            c = [ci - p[i] * dd for i, ci in enumerate(c)]
            r = 0.0 if rnd.random() < 0.5 else r
        d = unit(rnd)
        if k % 6 == 0:
            d = [0.0, -1.0, 0.0]
        out.append([fbits(x) for x in sum(pl, []) + c + [r] + d])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-37078')
    cs = cases(random.Random(37))
    nl, NL = chr(10), chr(92) + 'n'
    body = ('static const uint32_t C[][19]={' + ','.join('{' + ','.join('0x%08xu' % v for v in c) + '}' for c in cs) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            # the view at 0x1000 (planes at +768 / +784 / +800), the sphere at 0x1400, the direction at 0x1500
            'for(int i=0;i<12;i++)store(0x1000+768+4*i,c[i]);for(int i=0;i<4;i++)store(0x1400+4*i,c[12+i]);for(int i=0;i<3;i++)store(0x1500+4*i,c[16+i]);' +
            'printf("CASE");for(int i=0;i<19;i++)printf(" %08x",c[i]);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r4.u32=0x1000;ctx.r5.u32=0x1400;ctx.r6.u32=0x1500;' +
            ('sub_%s(ctx,nullptr);' % fn) + 'printf(" -> %u' + NL + '",ctx.r3.u32);}}' + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'casc_planes', vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    lines = [l for l in out.split(nl) if l]
    assert len(lines) == len(cs), (len(lines), out[:300])
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
