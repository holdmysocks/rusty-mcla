"""Run the original's effect cull box F-66872 offline, as the generated code has it, and record the box it writes, for
the port's test (crates/game/src/testdata/cull_sphere_reference.txt).

Inputs: r3 the effect instance (its matrix, rows x, y, z, position, at +176; its zoom at +500; its rule at +136, the rule's
CullSphere centre at +384 and CullRadius at +328), r4 the event instance (its zoom at +244); r5 / r6 where the box's min
and max go (four lanes each). The start-up masks the image holds as zero are set as the running game has them
(0x82860950 the xyz mask, 0x82860930 (0, 0, 0, 1)).

  CASE <rows 16 words> <centre xyz> <radius> <event zoom> <instance zoom> -> <min 4 words> <max 4 words>

python cull_sphere_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(rnd):
    out = []
    for k in range(4000):
        a, b = rnd.uniform(-math.pi, math.pi), rnd.uniform(-1, 1) if k % 3 else 0.0
        ca, sa, cb, sb = math.cos(a), math.sin(a), math.cos(b), math.sin(b)
        x, y, z = [ca, 0.0, -sa], [sa * sb, cb, ca * sb], [sa * cb, -sb, ca * cb]
        if k % 7 == 0:  # flipped (the mirror flag), scaled
            x = [-v for v in x]
        pos = [rnd.uniform(-3000, 3000), rnd.uniform(-10, 120), rnd.uniform(-2500, 2500)]
        rows = x + [0.0] + y + [0.0] + z + [0.0] + pos + [1.0]
        c = [rnd.choice((0.0, rnd.uniform(-5, 5))) for _ in range(3)]
        r = rnd.choice((0.0, -1.0, rnd.uniform(0, 20), rnd.uniform(-3, 0), 1e-30))
        ez, iz = rnd.choice((1.0, rnd.uniform(0, 3), -0.5)), rnd.choice((1.0, 0.01, rnd.uniform(0, 3)))
        out.append([fbits(v) for v in rows + c + [r, ez, iz]])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-66872')
    cs = cases(random.Random(66))
    nl, NL = chr(10), chr(92) + 'n'
    body = ('static const uint32_t C[][22]={' + ','.join('{' + ','.join('0x%08xu' % v for v in c) + '}' for c in cs) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'for(int i=0;i<3;i++)store(0x82860950+4*i,0xFFFFFFFFu);store(0x8286095C,0);for(int i=0;i<3;i++)store(0x82860930+4*i,0);store(0x8286093C,0x3F800000u);' +
            # the instance at 0x1000 (matrix +176, zoom +500, rule +136 -> 0x1400: centre +384, radius +328), the event at 0x1800
            'for(int i=0;i<16;i++)store(0x1000+176+4*i,c[i]);store(0x1000+500,c[21]);store(0x1000+136,0x1400);' +
            'for(int i=0;i<3;i++)store(0x1400+384+4*i,c[16+i]);store(0x1400+396,0x7FC00000u);store(0x1400+328,c[19]);store(0x1800+244,c[20]);' +
            'printf("CASE");for(int i=0;i<22;i++)printf(" %08x",c[i]);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r5.u32=0x1A00;ctx.r6.u32=0x1A10;' +
            ('sub_%s(ctx,nullptr);' % fn) + 'printf(" ->");for(int i=0;i<8;i++)printf(" %08x",load(0x1A00+4*i));printf("' + NL + '");}}' + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'cull_sphere', vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    lines = [l for l in out.split(nl) if l]
    assert len(lines) == len(cs), (len(lines), out[:300])
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
