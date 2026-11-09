"""Run the original's node rectangle distance F-46547 (the road / junction vfunc 3) offline on random rectangles and
points: the pre-test, the guest atan2, the turn F-58339 (guest sine / cosine), the excess over the half sizes, the
r² test. Prints one case a line, the words of the node's centre x z, width, length, dir x z, the point x z and r ->
the result's +44 (within), +36 (d2) and whether +32 holds the node, for the test navsys::node_rect_tests.

    python tools/node_rect_reference.py --recomp D:/MCLA-static --image <image> --tool <mcla-tool> > crates/formats/src/rage/testdata/node_rect_ref.txt
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import memcap

memcap.apply()


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fw(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def word(rng, lo, hi):
    k = rng.random()
    if k < 0.03:
        return rng.choice([0, 0x80000000, 0x00000001, 0x807FFFFF, 0x7FC00000, 0x7F800000, 0xFF800000, 0x3F800000])
    return fw(rng.uniform(lo, hi))


def case(rng):
    cx, cz = rng.uniform(-3000, 3000), rng.uniform(-3000, 3000)
    a = rng.uniform(-math.pi, math.pi)
    k = rng.random()
    dx, dz = (math.sin(a), math.cos(a)) if k < 0.8 else rng.choice([(0.0, 1.0), (1.0, 0.0), (0.0, 0.0), (-0.0, -1.0), (0.0, -0.0)])
    w, ln = rng.uniform(2, 40), rng.uniform(2, 200)
    reach = 0.6 * ln + 30
    px, pz = cx + rng.uniform(-reach, reach), cz + rng.uniform(-reach, reach)
    r = rng.choice([rng.uniform(0, 30), 0.0, 5.0, 50.0])
    ws = [fw(cx), fw(cz), fw(w), fw(ln), fw(dx), fw(dz), fw(px), fw(pz), fw(r)]
    if rng.random() < 0.1:
        ws[rng.randrange(9)] = word(rng, -100, 100)
    return ws


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, default=Path('work_node_rect'))
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--n', type=int, default=4000)
    a = ap.parse_args()
    idx = index(a.recomp)
    fn = addr(a.tool, 'F-46547')
    rng = random.Random(46547)
    cases = [case(rng) for _ in range(a.n)]
    arr = ','.join(','.join('0x%08Xu' % w for w in c) for c in cases)
    body = ('static const uint32_t cases[] = {%s};\n' % arr +
            'auto be = [](uint32_t a, uint32_t v){store(a, v);};\n'
            'auto rd = [](uint32_t a){return load(a);};\n'
            'for (size_t k = 0; k < sizeof(cases) / 4; k += 9) {\n'
            '  const uint32_t *q = cases + k; memset(objects, 0x5A, sizeof(objects));\n'
            '  be(0x1000 + 52, q[0]); be(0x1000 + 56, q[1]); be(0x1000 + 60, q[2]); be(0x1000 + 64, q[3]);\n'
            '  be(0x1000 + 96, q[4]); be(0x1000 + 104, q[5]);\n'
            '  be(0x1200, q[6]); be(0x1204, 0); be(0x1208, q[7]); be(0x120C, q[8]);\n'
            '  Context c{}; c.r3.u32 = 0x1400; c.r4.u32 = 0x1000; c.r5.u32 = 0x1200; c.r1.u32 = 0x2800;\n'
            '  sub_%s(c, nullptr);\n'
            '  for (int j = 0; j < 9; j++) printf("%%08X ", q[j]);\n'
            '  printf("-> %%u %%08X %%u\\n", (unsigned)(rd(0x1400 + 44) >> 24), rd(0x1400 + 36), rd(0x1400 + 32) == 0x1000u ? 1u : 0u);\n'
            '}\n' % fn)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n' + body + '}\n')
    exe = build(idx, [f'sub_{fn}'], main_src, a.out, 'node_rect', vector=True)
    print('# node c.x c.z width len dir.x dir.z  p.x p.z r -> within d2 owner (F-46547)')
    print(run(exe, a.image), end='')


if __name__ == '__main__':
    main()
