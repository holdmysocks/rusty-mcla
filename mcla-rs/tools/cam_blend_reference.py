"""Run the cine-to-game camera blend step of the original offline (static recompilation, native, flat guest memory) and
record its results for the port's test (crates/game/src/testdata/cam_blend_ref.txt).

  BLEND <t> <rate> <dt> <from 16> <from fov> <to 16> <to fov> -> <t'> <out 16> <out fov>
        F-82215 (called through F-89300 by the viewport's camera update): the blend object's
        time +0 (-1 = idle) moves by rate +84 x dt while it is above -1, ends (-1) at 1; while it runs the output is the
        matrix blend F-45235 from the stored camera +16 to the target by (1 - ease(t)) / 2 and the fov lerp,
        else the target itself. Matrices are rows of 4 floats (rotation rows, translation row +48). Every callee runs
        unchanged.
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# F-50812 dispatches through a jump table (the recomp keeps ctr): give the shim's context that register
_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda: _shim().replace('struct Context {', 'struct Context { PPCRegister ctr;', 1)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def quat_mat(q):
    x, y, z, w = q
    return [[1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)],
            [2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w)],
            [2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y)]]


def rand_mat(rnd, near=None):
    if near is None:
        v = [rnd.gauss(0, 1) for _ in range(4)]
    else:
        v = [c + rnd.gauss(0, near[1]) for c in near[0]]
    n = math.sqrt(sum(c * c for c in v))
    q = [c / n for c in v]
    r = quat_mat(q)
    t = [rnd.uniform(-500, 500) for _ in range(3)]
    return q, [r[0][0], r[0][1], r[0][2], 0.0, r[1][0], r[1][1], r[1][2], 0.0, r[2][0], r[2][1], r[2][2], 0.0, t[0], t[1], t[2], 1.0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    step = addr(a.tool, 'F-82215')
    rnd = random.Random(31)
    rows = []
    for k in range(3000):
        t = rnd.choice([-1.0, -1.0, -0.5, 0.0, 0.0, 1.0, 0.999, rnd.uniform(-1, 1), rnd.uniform(0, 1), rnd.uniform(0, 1)])
        rate = rnd.choice([1.0, 1.0, 1.0, 0.5, 2.0, rnd.uniform(0, 4)])
        dt = rnd.choice([1 / 30, 1 / 60, 0.0, 0.1, rnd.uniform(0, 0.2)])
        q0, m0 = rand_mat(rnd)
        _, m1 = rand_mat(rnd, None if rnd.random() < 0.5 else (q0 if rnd.random() < 0.5 else [-c for c in q0], rnd.choice([0.0, 1e-4, 0.01, 0.3])))
        f0, f1 = rnd.uniform(0.3, 1.4), rnd.uniform(0.3, 1.4)
        rows.append([fbits(x) for x in [t, rate, dt] + m0 + [f0] + m1 + [f1]])
    n = len(rows[0])
    table = 'static const uint32_t CASES[][%d]={' % n + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n'
    head = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
    # blend object at 0x1000 (t +0, from +16, from fov +80, rate +84), out matrix 0x1100, out fov 0x1180, target 0x1200
    body = table + head + (
        'for(size_t k=0;k<sizeof(CASES)/sizeof(CASES[0]);k++){const uint32_t*c=CASES[k];memset(objects,0,sizeof(objects));'
        'store(0x1000,c[0]);store(0x1054,c[1]);for(int i=0;i<16;i++)store(0x1010+4*i,c[3+i]);store(0x1050,c[19]);'
        'for(int i=0;i<16;i++)store(0x1200+4*i,c[20+i]);'
        'uint32_t w[2]={c[36],c[2]};float fv[2];memcpy(fv,w,8);'
        'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r5.u32=0x1180;ctx.r6.u32=0x1200;ctx.f1.f64=fv[0];ctx.f2.f64=fv[1];ctx.r1.u32=0x2800;'
        '%s(ctx,nullptr);'
        'printf("BLEND");for(int i=0;i<%d;i++)printf(" %%08x",c[i]);printf(" %%08x",load(0x1000));'
        'for(int i=0;i<16;i++)printf(" %%08x",load(0x1100+4*i));printf(" %%08x\\n",load(0x1180));}}\n') % (step, n)
    a.ref.write_text(run(build(idx, [step], body, a.out, 'camblend', vector=True, scratch_end=0x3000), a.image))


if __name__ == '__main__':
    main()
