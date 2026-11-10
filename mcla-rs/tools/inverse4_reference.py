"""The 4 x 4 inverse by cofactors run offline (generated code unchanged; scalar floating point, 16-byte vector moves, so
vector mode): its determinant F-62799 (r3 the matrix, returns f1), the inverse in place F-25750 (r3 the matrix; calls
the determinant, divides 1 by it with no guard) and the inverse into another matrix F-25548 (r3 out, r4 the matrix;
the same arithmetic with the stores direct). Writes <out-dir>/inverse4_ref.txt: "m16 -> det | inv16 | inv16'"
(hex bits; the out matrix pre-filled with garbage, every word printed)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def rot(rnd):
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(c * c for c in q))
    x, y, z, w = (c / n for c in q)
    return [[1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)],
            [2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w)],
            [2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y)]]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f_det, f_inv, f_to = (addr(a.tool, i) for i in ('F-62799', 'F-25750', 'F-25548'))
    rnd = random.Random(4)
    nan, inf = float('nan'), float('inf')
    cases = []
    for i in range(6000):
        k = i % 6
        if k == 0:      # a general matrix
            m = [rnd.uniform(-3, 3) for _ in range(16)]
        elif k == 1:    # a rigid frame (the car's / a bone's matrix): rotation, place, w column 0 0 0 1
            r = rot(rnd)
            m = [r[0][0], r[0][1], r[0][2], 0.0, r[1][0], r[1][1], r[1][2], 0.0, r[2][0], r[2][1], r[2][2], 0.0] + [rnd.uniform(-3000, 3000) for _ in range(3)] + [1.0]
        elif k == 2:    # a scaled frame (the bumper's 0.9995 axes) a little off square
            r = rot(rnd)
            s = rnd.uniform(0.2, 3)
            q = [r[i][j] * s + rnd.uniform(-1e-3, 1e-3) for i in range(3) for j in range(3)]
            m = q[0:3] + [0.0] + q[3:6] + [0.0] + q[6:9] + [0.0] + [rnd.uniform(-50, 50) for _ in range(3)] + [1.0]
        elif k == 3:    # singular: a repeated row, a zero row or column, a rank one
            m = [rnd.uniform(-3, 3) for _ in range(16)]
            j = rnd.randrange(3)
            if j == 0:
                m[4:8] = m[0:4]
            elif j == 1:
                m[8:12] = [0.0] * 4
            else:
                for r in range(4):
                    m[4 * r + 1] = 0.0
        elif k == 4:    # the identity, diagonals, permutations
            m = [0.0] * 16
            perm = list(range(4))
            if i % 12 != 4:
                rnd.shuffle(perm)
            for r in range(4):
                m[4 * r + perm[r]] = rnd.choice((1.0, -1.0, 2.0, 0.5))
        else:           # values over many magnitudes, some NaN / infinite
            m = [rnd.choice((-1, 1)) * 10 ** rnd.uniform(-3, 3) for _ in range(16)]
            if i % 30 == 5:
                m[rnd.randrange(16)] = rnd.choice((nan, inf, -inf))
        cases.append(m)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in cases)
    body = r'''
static const uint32_t C[][16]={
@ROWS@
};
uint32_t g=0x9E3779B9u;
auto garb=[&](uint32_t from,int n){for(int k=0;k<n;k++){g=g*1664525u+1013904223u;store(from+4*k,(g&0x3FFFFFFF)|0x20000000u);}};
auto pr=[&](uint32_t a,int n){for(int k=0;k<n;k++)printf(" %08x",load(a+4*k));};
for(auto&c:C){
  memset(objects,0,sizeof(objects));
  garb(0x2800,0x180);for(int k=0;k<16;k++)store(0x1000+4*k,c[k]);
  {Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_@DET@(ctx,nullptr);float d=(float)ctx.f1.f64;uint32_t u;memcpy(&u,&d,4);printf("%08x",u);}
  for(int k=0;k<16;k++)store(0x1100+4*k,c[k]);
  {Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1100;sub_@INV@(ctx,nullptr);printf(" |");pr(0x1100,16);}
  garb(0x1200,16);
  {Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1200;ctx.r4.u32=0x1000;sub_@TO@(ctx,nullptr);printf(" |");pr(0x1200,16);
   for(int k=0;k<16;k++)if(load(0x1000+4*k)!=c[k])printf(" INPUT_CHANGED");}
  printf("@NL@");
}
'''
    body = body.replace('@ROWS@', rows).replace('@NL@', NL).replace('@DET@', f_det).replace('@INV@', f_inv).replace('@TO@', f_to)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + f_det, 'sub_' + f_inv, 'sub_' + f_to], main_src, a.out_dir, 'inverse4', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    assert not any('INPUT_CHANGED' in o for o in out)
    lines = ['%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'inverse4_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
