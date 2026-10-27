"""The original's colour conversions run offline: F-47840 (rgb -> hsv in place, 3 floats at the object). Unchanged generated code on a flat guest memory (recomp_closure).
Writes map_hsv_ref.txt: lines "R r g b h s v" (input then the original's output) as f32 bit patterns (hex)."""
import argparse
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
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    rh = addr(a.tool, 'F-47840')
    rnd = random.Random(11)
    vals = [0.0, 1.0, 0.5, 0.25, 0.75, 1e-7, 1.0 / 3.0, 2.0 / 3.0, 0.999]
    rgb = [(x, y, z) for x in vals[:6] for y in vals[:6] for z in vals[:6]]
    rgb += [tuple(rnd.random() for _ in range(3)) for _ in range(4000)]
    rgb += [tuple(rnd.choice(vals) for _ in range(3)) for _ in range(500)]
    hsv = [(h, s, v) for h in (-0.2, 0.0, 1.0 / 6, 1.0 / 3, 0.5, 2.0 / 3, 5.0 / 6, 0.999999, 1.0, 1.5) for s in (0.0, 0.5, 1.0) for v in (0.0, 0.7, 1.0)]
    hsv += [(rnd.uniform(-0.3, 1.3), rnd.random(), rnd.random()) for _ in range(4000)]
    body = ''
    for c in rgb:
        body += ('{Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2800;store(0x1000,0x%08x);store(0x1004,0x%08x);store(0x1008,0x%08x);sub_%s(ctx,nullptr);'
                 'printf("R %%08x %%08x %%08x %%08x %%08x %%08x\\n",0x%08x,0x%08x,0x%08x,load(0x1000),load(0x1004),load(0x1008));}\n'
                 % (fb(c[0]), fb(c[1]), fb(c[2]), rh, fb(c[0]), fb(c[1]), fb(c[2])))
    src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n' + body + '}\n'
    exe = build(idx, [f'sub_{rh}'], src, a.out, 'map_hsv')
    a.out_dir.mkdir(parents=True, exist_ok=True)
    out = run(exe, a.image)
    (a.out_dir / 'map_hsv_ref.txt').write_text(out)
    print(len(out.splitlines()), 'cases')


if __name__ == '__main__':
    main()
