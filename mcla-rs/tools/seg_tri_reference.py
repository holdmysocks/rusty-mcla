"""Run the original's segment-vs-triangle test F-65698 offline (the generated code as it is; it calls nothing) and record
its answer for the port's test (crates/game/src/propfx.rs, testdata/seg_tri_reference.txt).

  SEG <a x y z> <b x y z> <v0 x y z> <v1 x y z> <v2 x y z> -> <0|1>

The cases: segments through, beside, along and short of random triangles, degenerate triangles (a zero normal, two
equal corners), segments in the triangle's plane, ends on the plane, hits on the edges and corners (u, v or u + v at
exactly 0 or 1), each axis the largest of the normal and ties between them, and NaN / inf words. (The harness does not
model the console's denormal flushing, so no case is made of denormals.)

python seg_tri_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
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


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def cases(rnd):
    out = []
    S = [0.0, -0.0, 1.0, -1.0, 0.5, 2.0, float('nan'), float('inf'), -float('inf')]
    for k in range(8000):
        kind = k % 8
        tri = [[rnd.uniform(-3, 3) for _ in range(3)] for _ in range(3)]
        if kind == 1:  # axis-aligned triangles: one normal axis exactly largest, or ties (45 degree planes)
            ax = rnd.randrange(3)
            h = rnd.choice((0.0, 0.2, -1.5))
            tri = [[rnd.choice((-1.0, 0.0, 1.0, 2.0)) for _ in range(3)] for _ in range(3)]
            for c in tri:
                c[ax] = h
            if rnd.random() < 0.3:  # tilt to a tie
                o = (ax + 1) % 3
                for c in tri:
                    c[ax] = c[o]
        elif kind == 2:  # degenerate: two equal corners or collinear
            tri[2] = list(tri[rnd.randrange(2)]) if rnd.random() < 0.5 else [(tri[0][i] + tri[1][i]) * 0.5 for i in range(3)]
        # the segment
        bary = [rnd.uniform(-0.3, 1.3), rnd.uniform(-0.3, 1.3)]
        if kind == 3:  # on the edges / corners exactly
            bary = rnd.choice(([0.0, rnd.uniform(0, 1)], [rnd.uniform(0, 1), 0.0], [0.5, 0.5], [0.25, 0.75], [1.0, 0.0], [0.0, 1.0], [0.0, 0.0]))
        p = [tri[0][i] + bary[0] * (tri[1][i] - tri[0][i]) + bary[1] * (tri[2][i] - tri[0][i]) for i in range(3)]
        dvec = [rnd.uniform(-2, 2) for _ in range(3)]
        lo, hi = rnd.choice(((-0.5, 0.5), (0.0, 1.0), (-1.0, 0.0), (0.2, 1.2), (-1.3, -0.1), (0.0, 0.0)))
        if kind == 4:  # in the plane: d along the triangle
            dvec = [(tri[1][i] - tri[0][i]) * rnd.uniform(-1, 1) + (tri[2][i] - tri[0][i]) * rnd.uniform(-1, 1) for i in range(3)]
        a = [p[i] + lo * dvec[i] for i in range(3)]
        b = [p[i] + hi * dvec[i] for i in range(3)]
        if kind == 5:  # ends exactly on the plane: a or b is a corner or the point
            if rnd.random() < 0.5:
                a = list(rnd.choice((tri[0], tri[1], tri[2], p)))
            else:
                b = list(rnd.choice((tri[0], tri[1], tri[2], p)))
        if kind == 6:  # special words
            vs = a + b + sum(tri, [])
            for _ in range(rnd.randrange(1, 3)):
                vs[rnd.randrange(15)] = rnd.choice(S)
            a, b, tri = vs[0:3], vs[3:6], [vs[6:9], vs[9:12], vs[12:15]]
        if kind == 7:  # far out, big and small triangles
            s = rnd.choice((1e-3, 100.0, 3000.0))
            a, b = [x * s for x in a], [x * s for x in b]
            tri = [[x * s for x in c] for c in tri]
        out.append([fbits(f32(x)) for x in a + b + sum(tri, [])])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-65698')
    cs = cases(random.Random(65698))
    nl, NL = chr(10), chr(92) + 'n'
    body = ('static const uint32_t C[][15]={' + ','.join('{' + ','.join('0x%08xu' % v for v in c) + '}' for c in cs) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            # a, b, v0, v1, v2 as vectors at 0x1000, 0x1010 .. 0x1040 (w = 0)
            'for(int j=0;j<5;j++)for(int i=0;i<3;i++)store(0x1000+16*j+4*i,c[3*j+i]);' +
            'printf("SEG");for(int i=0;i<15;i++)printf(" %08x",c[i]);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1010;ctx.r5.u32=0x1020;ctx.r6.u32=0x1030;ctx.r7.u32=0x1040;' +
            ('sub_%s(ctx,nullptr);' % fn) + 'printf(" -> %u' + NL + '",ctx.r3.u32);}}' + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'seg_tri', vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    lines = [l for l in out.split(nl) if l]
    assert len(lines) == len(cs), (len(lines), out[:300])
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(nl.join(lines) + nl)
    print(len(lines), 'cases', sum(l.endswith('1') for l in lines), 'hits')


if __name__ == '__main__':
    main()
