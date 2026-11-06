"""The walker spawner's random point of a polygon F-76960 run offline: unchanged generated code (vector mode) on flat guest
memory. The polygon's corners come from the vertex fetch F-36421, stubbed (it writes the case's n corners as
vec4s into the caller's buffer and returns n; its own pointer chasing is not run). The two draws are the function's own
(the global MWC, twice: 23-bit draws times 2^-23); each case sets the MWC state and gets it back advanced.
Cases: triangles and quads of walkable sizes (1 - 60 m), tilted or not, anywhere on the map, random MWC states.
Writes <out-dir>/random_point_ref.txt, one case a line: "n corners(xyz each, hex) mwc_lo mwc_carry -> x y z lo carry"."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += 'static uint32_t SV_[4][4];static int SN_;\n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    ad = addr(a.tool, 'F-76960')
    helper = addr(a.tool, 'F-36421')
    rnd = random.Random(76960)
    cases = []
    for _ in range(a.cases):
        n = rnd.choice((3, 4, 4))
        c = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        size = rnd.choice((1.0, 3.0, 8.0, 20.0, 60.0, rnd.uniform(0.5, 60)))
        tilt = rnd.choice((0.0, 0.0, 0.1, 1.0))
        pts = []
        for i in range(n):
            ang = (i + rnd.uniform(-0.2, 0.2)) * 6.2831853 / n + rnd.uniform(0, 0.3)
            r = size * rnd.uniform(0.4, 1.0)
            import math
            pts.append([f32(c[0] + math.cos(ang) * r), f32(c[1] + rnd.uniform(-1, 1) * tilt * size * 0.1), f32(c[2] + math.sin(ang) * r)])
        cases.append((n, pts, rnd.getrandbits(32), rnd.getrandbits(32)))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = []
    for n, pts, lo, ca in cases:
        ws = [n] + [fb(x) for p in pts for x in p] + [0] * (3 * (4 - n)) + [lo, ca]
        rows.append('{%s},' % ','.join('0x%08x' % w for w in ws))
    stubs = {'sub_' + helper: 'int n=SN_;for(int i=0;i<n;i++)for(int k=0;k<4;k++)store(ctx.r6.u32+16*i+4*k,SV_[i][k]);ctx.r3.u64=n;'}
    body = ('static const uint32_t C[][1+12+2]={' + nl + nl.join(rows) + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));SN_=int(c[0]);' +
            'for(int i=0;i<4;i++){for(int k=0;k<3;k++)SV_[i][k]=c[1+3*i+k];SV_[i][3]=0;}' +
            'store(0x1100+4*524,0x1400);store(0x1404,0x1500);store(0x1504,0x1600);store((0x8286u<<16|0x0890),c[13]);store((0x8286u<<16|0x0894),c[14]);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r5.u32=0;' +
            ('sub_%s(ctx,nullptr);printf("%%08x %%08x %%08x %%08x %%08x%s",load(0x1000),load(0x1004),load(0x1008),load((0x8286u<<16|0x0890)),load((0x8286u<<16|0x0894)));}' % (ad, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.out_dir, 'random_point', stubs=stubs, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (n, pts, lo, ca), o in zip(cases, out):
        lines.append('%d %s %08x %08x -> %s' % (n, ' '.join('%08x' % fb(x) for p in pts for x in p), lo, ca, o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that would read as addresses of the original are split by '_'; the test removes it)
    (a.out_dir / 'random_point_ref.txt').write_text(re.sub(r'(82[0-9a-f]{2})([0-9a-f]{4})', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
