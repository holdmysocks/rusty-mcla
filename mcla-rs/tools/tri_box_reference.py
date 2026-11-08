"""The ground triangle against a wheel's box F-55826 run offline (generated code unchanged, vector mode, the recomp's own
-msse4.1 -mssse3; a register's lane k is the host's f32[3 - k]): v1..v3 the triangle, v4 its normal, v5 the box's
centre, v6 its half sizes, r3 the box's three axes (rows at +0 / +16 / +32); r3 out (1 = no axis separates).

The xyz mask 0x82860950 is filled as the game's start-up does; the zero vector it is compared with is zero in the image.
Cases: boxes far from the origin and near it, triangles round them, and triangles put on a face of the box to within a
few ulps (the cases where the rounding of the projections decides). No NaNs (the recomp's vcmpbfp / vminfp do not
give the hardware's NaN answers).

Writes <out-dir>/tri_box_ref.txt, one case a line: "words.. -> r" (22 hex float bits: v1 v2 v3 v4 v5 v6 axes)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure

_vector_shim = recomp_closure.vector_shim


def _shim():
    s = _vector_shim()
    old = 'struct Condition {\n    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    return s.replace(old, '#include <simde/x86/avx2.h>\nstruct Condition {\n    bool so{}, eq{}, lt{}, gt{};\n'
                     '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)


recomp_closure.vector_shim = _shim


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def rot(rnd):
    a, b, c = (rnd.uniform(-math.pi, math.pi) for _ in range(3))
    ca, sa, cb, sb, cc, sc = math.cos(a), math.sin(a), math.cos(b), math.sin(b), math.cos(c), math.sin(c)
    r = [[cb * cc, cb * sc, -sb], [sa * sb * cc - ca * sc, sa * sb * sc + ca * cc, sa * cb], [ca * sb * cc + sa * sc, ca * sb * sc - sa * cc, ca * cb]]
    if rnd.random() < 0.3:
        r = [[1.0, 0, 0], [0, 1.0, 0], [0, 0, 1.0]]
    return r


def cases():
    rnd = random.Random(71)
    out = []
    for i in range(24000):
        far = rnd.choice((0.0, 50.0, 3000.0))
        c = [rnd.uniform(-far, far) for _ in range(3)]
        ax = rot(rnd)
        h = [rnd.uniform(0.05, 2.0) for _ in range(3)]
        s = rnd.choice((0.5, 2.0, 5.0))
        tri = [[c[k] + rnd.uniform(-s, s) * (1 + h[k]) for k in range(3)] for _ in range(3)]
        if i % 2 == 0:
            # one vertex on a face of the box (to within a few ulps), the others outside it on that side or across
            j, sg, on = rnd.randrange(3), rnd.choice((-1, 1)), rnd.randrange(3)
            for v in range(3):
                d = [rnd.uniform(-1, 1) * h[k] for k in range(3)]
                d[j] = sg * h[j] * (1 + rnd.choice((0.0, 1e-7, -1e-7, 3e-7)) if v == on else 1 + rnd.uniform(0, 1))
                tri[v] = [c[k] + sum(d[m] * ax[m][k] for m in range(3)) for k in range(3)]
        tri = [[f32(x) for x in v] for v in tri]
        e1 = [tri[1][k] - tri[0][k] for k in range(3)]
        e2 = [tri[2][k] - tri[0][k] for k in range(3)]
        n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
        ln = math.sqrt(sum(x * x for x in n)) or 1.0
        n = [x / ln for x in n]
        if i % 5 == 0:
            # the plane test on its edge: the box's corner on the triangle's plane
            reach = sum(h[k] * abs(sum(n[m] * ax[k][m] for m in range(3))) for k in range(3))
            off = sum(n[m] * (tri[0][m] - c[m]) for m in range(3)) - math.copysign(reach, rnd.uniform(-1, 1))
            tri = [[f32(v[m] - n[m] * off * (1 + rnd.choice((0.0, 1e-7)))) for m in range(3)] for v in tri]
        out.append([x for v in tri for x in v] + n + c + h + [x for r in ax for x in r])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ft = addr(a.tool, 'F-55826')
    cs = cases()
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in cs)
    setv = ''.join('for(int k=0;k<3;k++)ctx.v%d.u32[3-k]=c[%d+k];ctx.v%d.u32[0]=0;' % (1 + i, 3 * i, 1 + i) for i in range(6))
    axes = 'for(int r=0;r<3;r++){for(int k=0;k<3;k++)store(0x1000+16*r+4*k,c[18+3*r+k]);store(0x1000+16*r+12,0);}'
    consts = 'for(int k=0;k<3;k++)store(0x82860950+4*k,0xFFFFFFFF);store(0x8286095C,0);'
    body = ('static const uint32_t C[][27]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;' + setv + axes +
            ('ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);printf("%%u' % ft) + NL + '",ctx.r3.u32);}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + consts + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + ft], main_src, a.out_dir, 'tri_box', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = ['%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'tri_box_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o == '1' for o in out), 'touch')


if __name__ == '__main__':
    main()
