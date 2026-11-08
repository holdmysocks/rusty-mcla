"""Run the original's cone half-width F-46735 offline (with the C library's tan it calls, as the generated code has them)
and record the results for the port's test (crates/game/src/render/testdata/cone_tan_reference.txt).

The lamp copy's +88 (type byte), +68 (cone angle, F-19533 already scaled by 1.077) and +72 (penumbra plus the global)
are set per case; the result f1 is written as float bits.

  CONE <type> <cone hex> <penumbra hex> <result hex>

python cone_tan_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file>
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


def cases(rnd):
    out = []
    for k in range(6000):
        t = 1 if k % 10 else rnd.choice((0, 2, 3))
        m = k % 6
        if m == 0:  # the files' angles: ConeAngle x 1.077, Penumbra
            c, p = rnd.choice((125, 20, 90, 60, 45, 30, 100, 110, 150)) * 1.077, rnd.choice((-50, 0, 5, 10, -10, 20))
        elif m == 1:
            c, p = rnd.uniform(0, 180), rnd.uniform(-90, 90)
        elif m == 2:
            c, p = rnd.uniform(0, 400), rnd.uniform(-10, 100)
        elif m == 3:  # near the tan's pole (90 degrees half angle at 360)
            c, p = 360 + rnd.uniform(-1, 1) * 10 ** rnd.uniform(-5, 0), 0.0
        elif m == 4:
            c, p = rnd.uniform(-50, 50), rnd.choice((-0.0, 0.0, -1e-30, rnd.uniform(-1, 1)))
        else:
            c, p = rnd.uniform(0, 3000), rnd.uniform(-3000, 3000)
        out.append((t, fbits(c), fbits(p)))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-46735')
    cs = cases(random.Random(46))
    nl, NL = chr(10), chr(92) + 'n'
    body = ('static const uint32_t C[][3]={' + ','.join('{%d,0x%08xu,0x%08xu}' % c for c in cs) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));pointer(0x1000+88)[0]=uint8_t(c[0]);store(0x1000+68,c[1]);store(0x1000+72,c[2]);' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);' % fn) +
            'float f=float(ctx.f1.f64);uint32_t b;memcpy(&b,&f,4);printf("CONE %u %08x %08x %08x' + NL + '",c[0],c[1],c[2],b);}}' + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'cone_tan', vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    lines = [l for l in out.split(nl) if l]
    assert len(lines) == len(cs), (len(lines), len(cs))
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
