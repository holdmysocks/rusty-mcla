"""Run the garage camera's polar placement of the original offline (static recompilation, native, flat guest memory)
and record its results for the port's test (crates/game/src/testdata/garage_place_ref.txt).

  GP <case words> -> <rows +16 .. +76: 16 words>
        F-74130 in mode 1 (+216 = 1, the garage's): the heading atan2 of the target matrix's z axis (x, z) (the C
        library's, F-65975 as `guest_atan2`), the azimuth +200 + that, the rows of the zxy angles (-incline +204,
        azimuth, roll +208) by F-62659 through the basis builder, row 3 = row 2 x dist +196 + the target matrix's row 3,
        then +68 (its y) += height +192. Nothing stubbed.

Case words: matrix 16 (rows 0..3 at [+144]), height, dist, azimuth, incline, roll = 21.

Function names come from `mcla-tool registry addr`. Usage:
  python mcla-rs/tools/garage_place_reference.py --recomp D:\\MCLA-static --image work/2026-10-10/original-image.bin \\
      --out work/<session>/gplace --ref mcla-rs/crates/game/src/testdata/garage_place_ref.txt --tool <mcla-tool.exe>
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


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


CAM, MAT, FR = 0x1000, 0x1200, 0x1300
NW = 21


def case(rnd):
    yaw = rnd.choice([0.0, math.pi, -math.pi / 2, rnd.uniform(-math.pi, math.pi), rnd.uniform(-math.pi, math.pi)])
    tilt = rnd.choice([0.0, 0.0, rnd.uniform(-0.1, 0.1)])
    c, s, ct, st = math.cos(yaw), math.sin(yaw), math.cos(tilt), math.sin(tilt)
    m = [c, 0.0, -s, 0.0, s * st, ct, c * st, 0.0, s * ct, -st, c * ct, 0.0,
         rnd.uniform(-2000, 2000), rnd.uniform(-5, 80), rnd.uniform(-2000, 2000), rnd.choice([0.0, 1.0])]
    height = rnd.choice([0.0, 1.5, rnd.uniform(-0.5, 2.5)])
    dist = rnd.choice([0.5, 3.0, rnd.uniform(0.3, 9)])
    az = rnd.choice([0.0, math.pi, rnd.uniform(-7, 7), rnd.uniform(-math.pi, math.pi)])
    inc = rnd.choice([0.0, 0.05, rnd.uniform(-0.2, 1.2), rnd.uniform(0.015, 0.275)])
    roll = rnd.choice([0.0, 0.0, rnd.uniform(-0.3, 0.3)])
    return [fb(x) for x in m + [height, dist, az, inc, roll]]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    idx = index(a.recomp)
    place = addr(a.tool, 'F-74130')
    rnd = random.Random(1119)
    cases = [case(rnd) for _ in range(a.cases)]
    table = 'static const uint32_t CS[][%d]={' % NW + ','.join('{' + ','.join('0x%08xu' % v for v in c) + '}' for c in cases) + '};\n'
    body = table + (
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
        ' for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFFu);store(0x82860930+4*k,0);}store(0x8286095C,0);store(0x8286093C,0x3F800000u);\n'
        'for(size_t k=0;k<sizeof(CS)/sizeof(CS[0]);k++){const uint32_t*h=CS[k];memset(pointer(0x1000),0,0x400);\n'
        ' for(int i=0;i<16;i++)store(%d+4*i,h[i]);store(%d,%d);store(%d,1);\n'
        ' for(int i=0;i<5;i++)store(%d+4*i,h[16+i]);\n'
        '  Context ctx{};ctx.r3.u32=%d;ctx.r1.u32=0x3f00;sub_%s(ctx,nullptr);\n'
        '  printf("GP");for(int i=0;i<%d;i++)printf(" %%08x",h[i]);printf(" ->");for(int i=0;i<16;i++)printf(" %%08x",load(%d+4*i));'
        'printf("\\n");}}\n') % (MAT, CAM + 144, MAT, CAM + 216, CAM + 192, CAM, place[4:], NW, CAM + 16)
    exe = build(idx, [place], body, a.out, 'gplace', {}, vector=True, scratch_end=0x4000)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
