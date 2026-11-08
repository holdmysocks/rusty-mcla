"""A point carried through a frame, F-88353 (the rider x vehicle collide's helper), run offline (generated code
unchanged, vector mode, the recomp's -msse4.1 -mssse3; lane k of a register is the host's f32[3 - k]): v1..v4 the
frame's rows (the translation in v4), v5 the point; v1 out (xyz).

Cases: frames round the origin and 3 km out, points 0 .. 100 m, the w lanes of the inputs filled with garbage (the
routine must drop them).

Writes <out-dir>/xf_ref.txt, one case a line: "16 words (rows xyz, point xyz, w lanes) -> x y z" (hex float bits)."""
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
    fx = addr(a.tool, 'F-88353')
    rnd = random.Random(88353)
    cases = []
    for _ in range(8000):
        ang = [rnd.uniform(-math.pi, math.pi) for _ in range(3)]
        ca, sa, cb, sb, cc, sc = (f(x) for x in ang for f in (math.cos, math.sin))
        r = [[cb * cc, cb * sc, -sb], [sa * sb * cc - ca * sc, sa * sb * sc + ca * cc, sa * cb], [ca * sb * cc + sa * sc, ca * sb * sc - sa * cc, ca * cb]]
        far = rnd.choice((0.0, 50.0, 3000.0))
        t = [rnd.uniform(-far, far) for _ in range(3)]
        p = [rnd.uniform(-1, 1) * 10 ** rnd.uniform(-3, 2) for _ in range(3)]
        w = [rnd.choice((0.0, 1.0, rnd.uniform(-1e3, 1e3))) for _ in range(5)]
        cases.append([x for row in r + [t] + [p] for x in row] + w)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in cases)
    setv = ''.join('for(int k=0;k<3;k++)ctx.v%d.u32[3-k]=c[%d+k];ctx.v%d.u32[0]=c[%d];' % (1 + i, 3 * i, 1 + i, 15 + i) for i in range(5))
    body = ('static const uint32_t C[][20]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){Context ctx{};ctx.r1.u32=0x2F00;' + setv +
            ('sub_%s(ctx,nullptr);printf("%%08x %%08x %%08x' % fx) + NL + '",ctx.v1.u32[3],ctx.v1.u32[2],ctx.v1.u32[1]);}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fx], main_src, a.out_dir, 'xf', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'xf_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
