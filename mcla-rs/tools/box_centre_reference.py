"""The spawn box's centre F-97927 run offline (generated code unchanged, vector mode): t = clamp(mph / 60, 0, 1) by its
fsel steps, the offset fma(t, b - a, a) of the tune's pair +28 / +32 (or, with the flag byte r6 set, the across-the-road
pair +52 / +56), the centre focus (v2) + view (v1) x offset with the vector unit's multiply then add.

Cases: speeds -20 .. 200 mph and the clamp's edges, the flag 0 / 1, tune pairs from the shipped values (100, 0) and
random ones, views of unit and any length, focuses across the city.

Writes <out-dir>/box_centre_ref.txt, one case a line, all words hex: "mph flag a b pa pb vx vy vz fx fy fz -> cx cy cz"
(a word that reads as one of the original's addresses is written 82xx_xxxx)."""
import argparse
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
    fn = 'sub_' + addr(a.tool, 'F-97927')
    rnd = random.Random(97927)
    cases = []
    for _ in range(4000):
        mph = rnd.choice((rnd.uniform(-20, 200), rnd.uniform(0, 70), 0.0, 60.0, -0.0, 59.999996, 60.000004))
        pair = lambda: rnd.choice(((100.0, 100.0), (0.0, 0.0), (rnd.uniform(-50, 200), rnd.uniform(-50, 200))))
        (lo, hi), (plo, phi) = pair(), pair()
        ang = rnd.uniform(-3.2, 3.2)
        ln = rnd.choice((1.0, rnd.uniform(0.2, 3.0)))
        v = (ln * rnd.uniform(-1, 1), rnd.choice((0.0, rnd.uniform(-0.3, 0.3))), ln * rnd.uniform(-1, 1))
        if rnd.random() < 0.5:
            import math
            v = (math.sin(ang), 0.0, math.cos(ang))
        f = (rnd.uniform(-4000, 4000), rnd.uniform(-20, 200), rnd.uniform(-4000, 4000))
        cases.append((fb(mph), rnd.randrange(2), fb(lo), fb(hi), fb(plo), fb(phi), *map(fb, v), *map(fb, f)))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    T, O = 0x1000, 0x1100
    setv = lambda r, i: ''.join(f'ctx.{r}.u32[{3 - k}]=c[{i + k}];' for k in range(3)) + f'ctx.{r}.u32[0]=0;'
    body = ('static const uint32_t C[][12]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            f'store({T}+28,c[2]);store({T}+32,c[3]);store({T}+52,c[4]);store({T}+56,c[5]);' +
            f'Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32={O};ctx.r4.u32={T};ctx.r6.u32=c[1];' +
            'float m;memcpy(&m,&c[0],4);ctx.f1.f64=m;' + setv('v1', 6) + setv('v2', 9) +
            ('%s(ctx,nullptr);printf("%%08x %%08x %%08x' % fn) + NL + f'",load({O}),load({O}+4),load({O}+8));}}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fn], main_src, a.out_dir, 'box_centre', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'box_centre_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
