"""The road rectangle distance F-15989 run offline (generated code unchanged, vector mode, the CPU's dpps for vmsum3fp as
the recomp's x86 build has it): r3 the point, r4 the centre, r5 the direction (16-byte vectors), f1 the length, f2 the
width; the result f1.

Cases: points round and inside roads of the city's sizes, unit directions with and without a y part, directions of
any length, zero sizes.

Writes <out-dir>/road_box_dist_ref.txt, one case a line, all words hex: "px py pz cx cy cz dx dy dz len width -> dist"
(a word that reads as one of the original's addresses is written 82xx_xxxx)."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure

recomp_closure.EXTRA += '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10)


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
    fn = 'sub_' + addr(a.tool, 'F-15989')
    rnd = random.Random(15989)
    cases = []
    for _ in range(6000):
        c = (rnd.uniform(-4000, 4000), rnd.uniform(-20, 200), rnd.uniform(-4000, 4000))
        ang = rnd.uniform(-3.2, 3.2)
        dy = rnd.choice((0.0, 0.0, rnd.uniform(-0.2, 0.2)))
        d = (math.cos(ang), dy, math.sin(ang))
        if rnd.random() < 0.15:
            d = tuple(rnd.uniform(-2, 2) for _ in range(3))
        ln = rnd.choice((rnd.uniform(5, 400), rnd.uniform(0, 40), 0.0))
        w = rnd.choice((rnd.uniform(5, 60), rnd.uniform(0, 10), 0.0))
        r = rnd.choice((rnd.uniform(0, 30), rnd.uniform(0, 400), 0.0))
        pa = rnd.uniform(-3.2, 3.2)
        p = (c[0] + r * math.cos(pa), c[1] + rnd.uniform(-10, 10), c[2] + r * math.sin(pa))
        cases.append(tuple(map(fb, (*p, *c, *d, ln, w))))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in cs) for cs in cases)
    P, Cc, D = 0x1000, 0x1010, 0x1020
    body = ('static const uint32_t C[][11]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            ''.join(f'store({P}+{4 * k},c[{k}]);store({Cc}+{4 * k},c[{3 + k}]);store({D}+{4 * k},c[{6 + k}]);' for k in range(3)) +
            f'Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32={P};ctx.r4.u32={Cc};ctx.r5.u32={D};' +
            'float l,w;memcpy(&l,&c[9],4);memcpy(&w,&c[10],4);ctx.f1.f64=l;ctx.f2.f64=w;' +
            ('%s(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);printf("%%08x' % fn) + NL + '",u);}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fn], main_src, a.out_dir, 'road_box_dist', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in cs), o) for cs, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'road_box_dist_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
