"""The paint colours' HSV -> RGB (F-61251) run offline: the original's generated code whole and unchanged (its six-way
jump table included, the double fmod of the C library with it) on a colour of three floats it turns in place.
Cases: s = 0, the six sextants and their edges, negative hues, hues past 1, NaN / infinity, and random ones.
Writes <out-dir>/hsv_rgb_ref.txt, one case a line: the three input floats' bits, then the three output floats' bits (hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

COL = 0x40108000
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(n):
    rnd = random.Random(61251)
    out = []
    edges = [0.0, 1 / 6, 2 / 6, 0.5, 4 / 6, 5 / 6, 1.0, 0.99999994, 1e-8, -1e-8, -0.25, -1.0, -1.5, 1.25, 7.3, 1e7, -1e7]
    for h in edges:
        for s in (0.0, 0.5, 1.0, -0.0):
            out.append((bits(h), bits(s), bits(0.75)))
    for special in (0x7FC00000, 0x7F800000, 0xFF800000):
        out.append((special, bits(0.5), bits(0.5)))
    while len(out) < n:
        h = rnd.choice([rnd.random(), rnd.uniform(-3, 3), rnd.random() * 1e-3, 1 - rnd.random() * 1e-6])
        out.append((bits(h), bits(rnd.choice([rnd.random(), 1.0, rnd.uniform(-1, 2)])), bits(rnd.choice([rnd.random(), 1.0, rnd.uniform(0, 4)]))))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--n', type=int, default=3000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-61251')
    cs = cases(a.n)
    nl = chr(10)
    main_src = (
        'static const uint32_t V[]={' + ','.join('%du,%du,%du' % c for c in cs) + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(size_t i=0;i<sizeof(V)/12;i++){for(int k=0;k<3;k++)store(%du+4*k,V[3*i+k]);' % COL +
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;%s(ctx,nullptr);}' % (STACK, COL, fn) +
        'printf("%%08x %%08x %%08x %%08x %%08x %%08x\\n",V[3*i],V[3*i+1],V[3*i+2],load(%du),load(%du),load(%du));}}' % (COL, COL + 4, COL + 8) + nl)
    idx = index(a.recomp)
    exe = rd.build(idx, [fn], main_src, a.out_dir, 'hsv_rgb', image=rd.load_image(a.image))
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'hsv_rgb_ref.txt').write_text('# F-61251 on 3 floats in place: h s v -> out[0] out[1] out[2] (f32 bits)' + nl + out)
    print(len(out.splitlines()), 'cases')


if __name__ == '__main__':
    main()
