"""The breakable entry's inertia floor F-88418 run offline (generated code unchanged, scalar mode): r3 three floats; the
largest found by fcmpu (x < y, then y < z or x < z: a NaN or a tie keeps the earlier one), then every other one whose
x 20 is below the largest (bge is taken on unordered too) becomes the largest x 0.05.

Cases: shipped-like inertias (0.01 .. 50 each), one far smaller than the others, ties, zeros, negatives, NaN, inf,
subnormals.

Writes <out-dir>/inertia_floor_ref.txt, one case a line, hex: "x y z -> x y z"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

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
    fn = 'sub_' + addr(a.tool, 'F-88418')
    rnd = random.Random(88418)
    special = (0.0, -0.0, -1.0, float('nan'), float('inf'), 1e-40, 3e38)
    cases = []
    for _ in range(5000):
        big = rnd.uniform(0.01, 50)
        v = [rnd.choice((rnd.uniform(0.01, 50), big, big / rnd.uniform(1, 100), big / 20, rnd.uniform(0, 1))) for _k in range(3)]
        if rnd.random() < 0.15:
            v[rnd.randrange(3)] = rnd.choice(special)
        if rnd.random() < 0.1:
            v[rnd.randrange(3)] = v[rnd.randrange(3)]
        cases.append([fb(x) for x in v])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{0x%08x,0x%08x,0x%08x},' % tuple(c) for c in cases)
    body = ('static const uint32_t C[][3]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){for(int k=0;k<3;k++)store(0x1000+4*k,c[k]);Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            f'{fn}(ctx,nullptr);printf("%08x %08x %08x' + NL + '",load(0x1000),load(0x1004),load(0x1008));}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fn], main_src, a.out_dir, 'inertia_floor')
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%08x %08x %08x -> %s' % (*c, o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'inertia_floor_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
