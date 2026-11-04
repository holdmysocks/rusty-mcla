"""The parked tree's box-against-circle test (F-80185) run offline: the generated code whole and unchanged on flat guest
memory. Memory: the node 0x1000 (+52 / +56 min x / z, +60 / +64 max x / z), the circle 0x1100 (+0 x, +8 z, +12 radius).
Most cases put the radius a float step or two around either rounding of the squared distance, where the rounding decides.
Writes <out-dir>/box_circle_ref.txt, one case a line: min x, min z, max x, max z, x, z, r (float bits, hex), then '->'
and 1 when the circle meets the box, else 0."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-80185')
    rnd = random.Random(80185)
    cases = []
    for _ in range(20000):
        x0, z0 = rnd.uniform(-2000, 2000), rnd.uniform(-2000, 2000)
        w, h = rnd.choice((rnd.uniform(0.5, 400), 0.0)), rnd.uniform(0.5, 400)
        x1, z1 = x0 + w, z0 + h
        px = rnd.choice((rnd.uniform(x0 - 300, x1 + 300), x0, x1, rnd.uniform(x0, x1)))
        pz = rnd.choice((rnd.uniform(z0 - 300, z1 + 300), z0, z1, rnd.uniform(z0, z1)))
        x0, z0, x1, z1, px, pz = (f32(v) for v in (x0, z0, x1, z1, px, pz))
        dx = f32(px - x0) if px < x0 else (f32(px - x1) if px > x1 else None)
        dz = f32(pz - z0) if pz < z0 else (f32(pz - z1) if pz > z1 else None)
        d2x = f32(dx * dx) if dx is not None else 0.0
        # the two candidate sums: z fused onto x's part (one rounding of the exact sum, as the double fma then f32), and
        # z's product rounded first
        fused = f32(dz * dz + d2x) if dz is not None else d2x
        twice = f32(f32(dz * dz) + d2x) if dz is not None else d2x
        if rnd.random() < 0.6 and fused > 0:
            # a radius whose rounded square lands a step or two around either sum
            t = rnd.choice((fused, twice))
            r = bf(max(fb(f32(math.sqrt(t))) + rnd.randint(-2, 2), 0))
        else:
            r = rnd.uniform(0, 400)
        cases.append([fb(v) for v in (x0, z0, x1, z1, px, pz, r)])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    body = ('static const uint32_t C[][7]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));'
            'store(0x1000+52,c[0]);store(0x1000+56,c[1]);store(0x1000+60,c[2]);store(0x1000+64,c[3]);'
            'store(0x1100,c[4]);store(0x1100+8,c[5]);store(0x1100+12,c[6]);'
            'Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;'
            'sub_%s(ctx,nullptr);' % fn + ('printf("%%d%s",int(ctx.r3.u32&0xFF));}' % NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], main_src, a.out_dir, 'box_circle', stubs={}, scratch_end=0x80000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'box_circle_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o == '1' for o in out), 'meet')


if __name__ == '__main__':
    main()
