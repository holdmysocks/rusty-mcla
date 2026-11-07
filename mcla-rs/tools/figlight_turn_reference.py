"""Run the original's turn-towards helper offline (static recompilation, native, flat guest memory) and record its
results for the port's test (crates/game/src/render/testdata/figlight_turn_reference.txt).

  TURN <from x y z> <to x y z> <frac> <out x y z>   F-27981 (with F-80753 and F-10410): `from` turned towards `to`

python figlight_turn_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if n > 1e-3:
            return [x / n for x in v]


HEAD = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
MAX_CASES = 20000


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    turn = addr(a.tool, 'F-27981')[4:]
    rnd = random.Random(81)
    rows = []
    for k in range(4000):
        f, t = unit(rnd), unit(rnd)
        m = k % 8
        if m == 1:  # nearly the same way
            t = [f[i] + rnd.uniform(-1, 1) * 10.0 ** rnd.uniform(-7, -1) for i in range(3)]
        elif m == 2:  # nearly opposite
            t = [-f[i] + rnd.uniform(-1, 1) * 10.0 ** rnd.uniform(-7, -1) for i in range(3)]
        elif m == 3:  # near right angles (the |cos| < 0.001 cut)
            d = sum(f[i] * t[i] for i in range(3))
            t = [t[i] - d * f[i] + f[i] * rnd.uniform(-0.002, 0.002) for i in range(3)]
        elif m == 4:
            t = list(f)
        elif m == 5:
            t = [-x for x in f]
        frac = rnd.choice([1 - 0.94, 1 - 0.975, 1 - 0.94 ** 2, 1 - 0.975 ** 3, rnd.uniform(0, 1), 0.0, 1.0])
        rows.append([fbits(x) for x in f + t + [frac]])
    assert 0 < len(rows) <= MAX_CASES
    body = 'static const uint32_t C[][7]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n' + HEAD + (
        'for(auto&c:C){memset(objects,0,sizeof(objects));for(int i=0;i<3;i++){store(0x1000+4*i,c[i]);store(0x1100+4*i,c[3+i]);}'
        'store(0x100c,0u);store(0x110c,0u);float fr;memcpy(&fr,&c[6],4);'
        'Context ctx{};ctx.r4.u32=0x1000;ctx.r5.u32=0x1100;ctx.f1.f64=fr;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);'
        'printf("TURN");for(int i=0;i<7;i++)printf(" %%08x",c[i]);for(int i=0;i<3;i++)printf(" %%08x",load(0x1000+4*i));printf("\\n");}}\n') % turn
    out = run(build(idx, ['sub_' + turn], body, a.out, 'turn', vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)


if __name__ == '__main__':
    main()
