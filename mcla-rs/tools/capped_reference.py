"""A pool body's speed and spin caps run offline: F-80759 (velocity +272, momentum +304 against MaxSpeed +200) and
F-29166 (spin +288, angular momentum +320 against MaxAngSpeed +204), the generated code unchanged on flat guest memory
(the collider at 0x1000; the constants 0.5 and 1.0 read from the image).

Writes <out-dir>/capped_ref.txt, one case a line: "s|w vx vy vz px py pz max -> vx vy vz px py pz" (hex float bits)."""
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


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fs, fw = addr(a.tool, 'F-80759'), addr(a.tool, 'F-29166')
    rnd = random.Random(5)
    cases = []
    for _ in range(4000):
        mx = rnd.choice((500.0, 2 * math.pi, 50.0, 1.0, 0.0, rnd.uniform(0, 100)))
        s = mx * 10 ** rnd.uniform(-2, 2) if mx else 10 ** rnd.uniform(-6, 3)
        v = [rnd.uniform(-1, 1) * s for _ in range(3)]
        if rnd.random() < 0.1:
            v[rnd.randrange(3)] = 0.0
        p = [rnd.uniform(-1, 1) * 10 ** rnd.uniform(-3, 4) for _ in range(3)]
        cases.append((rnd.choice('sw'), v, p, mx))
    for mx in (500.0, 2 * math.pi):
        for v in ([mx, 0, 0], [0, -mx, 0], [0, 0, mx * 1.0000001], [mx / 3 ** 0.5] * 3, [0, 0, 0], [1e30, 0, 0], [float('inf'), 0, 0], [float('nan'), 1, 1]):
            for w in 'sw':
                cases.append((w, v, [1.0, -2.0, 3.0], mx))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%d,%s},' % (c[0] == 'w', ','.join('0x%08x' % fb(x) for x in c[1] + c[2] + [c[3]])) for c in cases)
    body = ('static const uint32_t C[][8]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));uint32_t o=c[0]?16:0;' +
            'for(int k=0;k<3;k++){store(0x1000+272+o+4*k,c[1+k]);store(0x1000+304+o+4*k,c[4+k]);}store(0x1000+200+(c[0]?4:0),c[7]);' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;if(c[0])sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);' % (fw, fs)) +
            'for(int k=0;k<3;k++)printf("%08x ",load(0x1000+272+o+4*k));for(int k=0;k<3;k++)printf(k<2?"%08x ":"%08x' + NL + '",load(0x1000+304+o+4*k));}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fs, 'sub_' + fw], main_src, a.out_dir, 'capped')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s %s -> %s' % (c[0], ' '.join('%08x' % fb(x) for x in c[1] + c[2] + [c[3]]), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'capped_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
