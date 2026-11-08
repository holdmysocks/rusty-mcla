"""Run the original's light table round trip offline: the packer F-49342 (a lamp into its 24-byte entry) and the fetch
F-31785 (the entry back into a light, through the set F-25852, stubbed here to print what it is given), as the
generated code has them. Records what comes back for the port's test
(crates/game/src/render/testdata/lamp_table_reference.txt).

The place goes through shorts on a scale and bias the game sets at run time (zero here, so it is not compared);
the direction, colour, cone and intensity through half floats, the penumbra through a byte.

  LAMP <dir x y z> <colour r g b> <intensity> <cone> <penumbra> -> <dir x y z> <colour r g b> <intensity> <cone> <penumbra>

python lamp_table_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file>
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


SPECIAL = [0.0, -0.0, 1.0, -1.0, 6.1035156e-05, 6.0e-05, 3.0e-05, 1e-8, 65504.0, 65520.0, 70000.0, 1e30, float('nan'), 0.147949, -0.988995]


def cases(rnd):
    out = []
    for k in range(4000):
        f = lambda lo, hi: rnd.choice(SPECIAL) if rnd.random() < 0.08 else rnd.uniform(lo, hi)
        d = [f(-1, 1) for _ in range(3)]
        c = [f(0, 2) for _ in range(3)]
        inten = rnd.choice((55.0, 80.0, 40.0, 0.0, f(0, 500)))
        cone = rnd.choice((125.0, 20.0, 90.0, 360.0, f(0, 400)))
        pen = rnd.choice((-50.0, 0.0, 30.0, 20.0, 60.0, -10.0, rnd.uniform(-127.5, 127.5), rnd.uniform(-300, 300), rnd.randrange(-128, 128) + rnd.choice((0.0, 0.5, -0.5))))
        out.append([fbits(x) for x in d + c + [inten, cone, pen]])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    pack, fetch, setf = (addr(a.tool, f) for f in ('F-49342', 'F-31785', 'F-25852'))
    cs = cases(random.Random(31))
    nl, NL = chr(10), chr(92) + 'n'
    # (the set's r5 colour vector, r6 direction, f1 intensity, f2 cone, f3 penumbra)
    stubs = {'sub_' + setf: ('for(int i=0;i<3;i++)printf(" %08x",load(ctx.r6.u32+4*i));for(int i=0;i<3;i++)printf(" %08x",load(ctx.r5.u32+4*i));'
                             'float g[3]={float(ctx.f1.f64),float(ctx.f2.f64),float(ctx.f3.f64)};uint32_t b[3];memcpy(b,g,12);'
                             'printf(" %08x %08x %08x' + NL + '",b[0],b[1],b[2]);')}
    body = ('static const uint32_t C[][9]={' + ','.join('{' + ','.join('0x%08xu' % v for v in c) + '}' for c in cs) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            # the direction at 0x1100, the colour at 0x1200, the place (zero) at 0x1300; the entry at 0x1400, the light at 0x1500
            'for(int i=0;i<3;i++){store(0x1100+4*i,c[i]);store(0x1200+4*i,c[3+i]);}' +
            'float g[3];memcpy(g,&c[6],12);' +
            'printf("LAMP");for(int i=0;i<9;i++)printf(" %08x",c[i]);printf(" ->");' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1400;ctx.r4.u32=1;ctx.r5.u32=0;ctx.r6.u32=0;ctx.r7.u32=0x1300;ctx.r8.u32=0x1200;ctx.r9.u32=0x1100;' +
            'ctx.f1.f64=g[0];ctx.f2.f64=g[1];ctx.f3.f64=g[2];' + ('sub_%s(ctx,nullptr);' % pack) +
            'Context cx{};cx.r1.u32=0x2F00;cx.r3.u32=0x1500;cx.r4.u32=0x1400;' + ('sub_%s(cx,nullptr);}}' % fetch) + nl)
    out = run(build(index(a.recomp), ['sub_' + pack, 'sub_' + fetch], body, a.out, 'lamp_table', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    lines = [l for l in out.split(nl) if l]
    assert len(lines) == len(cs), (len(lines), len(cs), out[:300])
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
