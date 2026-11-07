"""A box's moments F-30573 (f1 mass, f2..f4 the sides -> r7 three floats) and a body's inverse inertia in the world
F-16702 (r3 the three rows, v1 the inverse moments, r4 the three rows out) run offline: the generated code unchanged,
the second in vector mode (a register's lane k is the host's f32[3 - k]: the bytes reversed). Writes
<out-dir>/inertia_ref.txt: "box m x y z -> i0 i1 i2" and "world r0 r1 r2 (xyz each) inv (xyz) -> o0 o1 o2 (xyz each)"."""
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


def rotation(rnd):
    # rows of a random rotation (a unit quaternion), sometimes a little off square as a moving body's are
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(c * c for c in q))
    w, x, y, z = (c / n for c in q)
    r = [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)],
         [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
         [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]
    if rnd.random() < 0.2:
        r = [[c + rnd.uniform(-1e-3, 1e-3) for c in row] for row in r]
    return r


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fbx, fw = addr(a.tool, 'F-30573'), addr(a.tool, 'F-16702')
    rnd = random.Random(43)
    boxes = [[rnd.choice((4.0, rnd.uniform(0.1, 3000))), *(rnd.uniform(0.01, 6) for _ in range(3))] for _ in range(3000)]
    boxes += [[4.0, 2.25, 0.41625, 0.625], [0.0, 1, 1, 1], [1.0, 0, 0, 0]]
    worlds = []
    for i in range(3000):
        r = rotation(rnd) if i % 10 else [[1, 0, 0], [0, 1, 0], [0, 0, 1]]
        inv = [1 / rnd.uniform(0.05, 4000) for _ in range(3)]
        worlds.append([c for row in r for c in row] + inv)
    nl = chr(10)
    NL = chr(92) + 'n'
    rb = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in boxes)
    rw = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in worlds)
    fl = 'float r;uint32_t u;'
    body = ('static const uint32_t B[][4]={' + nl + rb + nl + '};' + nl + 'static const uint32_t W[][12]={' + nl + rw + nl + '};' + nl +
            'for(auto&c:B){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;ctx.r7.u32=0x1000;double*F[4]={&ctx.f1.f64,&ctx.f2.f64,&ctx.f3.f64,&ctx.f4.f64};' +
            'for(int k=0;k<4;k++){float x;uint32_t u=c[k];memcpy(&x,&u,4);*F[k]=x;}' +
            ('sub_%s(ctx,nullptr);printf("%%08x %%08x %%08x%s",load(0x1000),load(0x1004),load(0x1008));}' % (fbx, NL)) + nl +
            'for(auto&c:W){memset(objects,0,sizeof(objects));for(int i=0;i<3;i++)for(int k=0;k<3;k++)store(0x1000+16*i+4*k,c[3*i+k]);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;for(int k=0;k<3;k++)ctx.v1.u32[3-k]=c[9+k];ctx.v1.u32[0]=0;' +
            ('sub_%s(ctx,nullptr);for(int i=0;i<3;i++)for(int k=0;k<3;k++)printf(i+k<4?"%%08x ":"%%08x%s",load(0x1100+16*i+4*k));}' % (fw, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fbx, 'sub_' + fw], main_src, a.out_dir, 'inertia', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(boxes) + len(worlds), len(out)
    lines = ['box %s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(boxes, out)]
    lines += ['world %s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(worlds, out[len(boxes):])]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'inertia_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
