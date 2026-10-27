"""The night ambient map's cell lookup run offline: F-36356 (the cell index of a place) - unchanged generated code
on a flat guest memory. Object: min x / z at +0 / +8, max x / z at +16 / +24, cell +40, cells x / z +44 / +48; the place's x / z at +0 / +8.
Writes map_aomap_ref.txt: "minx minz maxx maxz cell nx nz px pz index" (floats as f32 bits, then -1 or the cell index)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

NL = chr(92) + 'n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    ad = addr(a.tool, 'F-36356')
    rnd = random.Random(3)
    sets = [(-3482.4773, -2449.0693, 2917.5227, 2350.9307, 6.25, 1024, 768),
            (-100.0, -50.0, 100.0, 60.0, 0.3, 700, 380),
            (0.0, 0.0, 50.0, 50.0, 0.7, 60, 60),
            (-10.5, 7.25, 40.0, 33.0, 0.1, 500, 260)]
    cases = []
    for p in sets:
        mx, mz, xx, xz, cell, nx, nz = [struct.unpack('>f', struct.pack('>f', v))[0] for v in p[:5]] + [p[5], p[6]]
        pts = []
        for _ in range(1500):
            pts.append((rnd.uniform(mx - 5, xx + 5), rnd.uniform(mz - 5, xz + 5)))
        for _ in range(600):
            kx, kz = rnd.randint(0, nx), rnd.randint(0, nz)
            ex, ez = mx + kx * cell, mz + kz * cell
            for dx in (0, 1, -1):
                bx = bf((fb(ex) + dx) & 0xffffffff)
                pts.append((bx, bf((fb(ez) + rnd.choice((0, 1, -1))) & 0xffffffff)))
        pts += [(mx, mz), (xx, xz), (xx, mz), (mx, xz), (mx - 1e-3, mz), (xx + 1e-3, mz)]
        for x, z in pts:
            cases.append((p, (mx, mz, xx, xz, cell, nx, nz), x, z))
    body = ''
    for p, (mx, mz, xx, xz, cell, nx, nz), x, z in cases:
        body += ('{memset(objects,0,sizeof(objects));store(0x1000,0x%08x);store(0x1008,0x%08x);store(0x1010,0x%08x);store(0x1018,0x%08x);store(0x1028,0x%08x);'
                 'store(0x102c,%d);store(0x1030,%d);store(0x1200,0x%08x);store(0x1208,0x%08x);Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;ctx.r1.u32=0x2800;'
                 'sub_%s(ctx,nullptr);printf("%%d%s",(int)ctx.r3.s32);}' % (fb(mx), fb(mz), fb(xx), fb(xz), fb(cell), nx, nz, fb(x), fb(z), ad, NL) + chr(10))
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + chr(10) + body + '}' + chr(10))
    exe = build(idx, ['sub_' + ad], main_src, a.out, 'map_aomap', vector=True)
    out = run(exe, a.image).split()
    assert len(out) == len(cases)
    lines = []
    for (p, (mx, mz, xx, xz, cell, nx, nz), x, z), o in zip(cases, out):
        lines.append('%08x %08x %08x %08x %08x %d %d %08x %08x %s' % (fb(mx), fb(mz), fb(xx), fb(xz), fb(cell), nx, nz, fb(x), fb(z), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'map_aomap_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
