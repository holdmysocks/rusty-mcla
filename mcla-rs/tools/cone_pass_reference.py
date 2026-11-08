"""Run the original's per-lamp cone setup F-19533 offline (with the lamp copy F-10025, the set F-25852, the edge F-51633,
the LightMatrix builder F-16203 with the half width F-46735, the 4x4 inverse F-25750 and the library cos / tan, as the
generated code has them; the two vector masks the startup fills are set first) and record every shader variable it sends, for the port's test
(crates/game/src/render/testdata/cone_pass_reference.txt).

The shader variable setter F-17339 is stubbed to print its handle (r5) and the words it is given (r6, r7 bytes).
The pass object holds the handles 8, 12, 16, 20, 40, 44, 48 at those offsets (so a printed handle names its field),
+52 the apex pull-back, +56 the longest cone, +64 the edge brightness. F-19533 writes 0.2 into +52 after use, so
each case sets it again.

  CASE <lamp words: pos x y z w, colour r g b a, dir x y z w, intensity, cone, penumbra> <pass +52 +56 +64> <f1 f2 f3>
  SET <handle> <words ...>      (one line per variable, in the order sent)

python cone_pass_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file>
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


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if n > 1e-3:
            return [x / n for x in v]


def cases(rnd):
    out = []
    for k in range(1500):
        pos = [rnd.uniform(-3000, 3000), rnd.uniform(-10, 120), rnd.uniform(-2500, 2500), 0.0]
        col = [rnd.choice((1.0, 0.9, rnd.uniform(0, 1))), rnd.choice((0.75, 0.8, rnd.uniform(0, 1))), rnd.choice((0.55, 0.65, rnd.uniform(0, 1))), 0.0]
        d = unit(rnd)
        if k % 3 == 0:  # the light files' Direction (straight down and out)
            d = [0.147949, -0.988995 * rnd.choice((1, -1)), 0.0]
        cone, pen = rnd.choice(((125.0, -50.0), (20.0, 0.0), (90.0, 10.0), (rnd.uniform(1, 200), rnd.uniform(-60, 30)),
                                (rnd.uniform(300, 400), rnd.uniform(-20, 20)), (125.0, rnd.uniform(-100, 100))))
        inten = rnd.choice((55.0, 80.0, 40.0, rnd.uniform(0, 120), 0.0))
        pull, longest, edge = rnd.choice((0.2, 0.0, rnd.uniform(0, 1))), rnd.choice((16.0, rnd.uniform(1, 30))), rnd.choice((0.6, 1.0, rnd.uniform(0.01, 3)))
        f1, f2, f3 = rnd.uniform(0, 2), rnd.uniform(0, 2), rnd.uniform(0, 2)
        out.append([fbits(x) for x in pos + col + d + [0.0, inten, cone, pen, pull, longest, edge, f1, f2, f3]])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, setter = addr(a.tool, 'F-19533'), addr(a.tool, 'F-17339')
    cs = cases(random.Random(19))
    nl, NL = chr(10), chr(92) + 'n'
    stubs = {'sub_' + setter: 'printf("SET %u",ctx.r5.u32);for(uint32_t i=0;i<ctx.r7.u32;i+=4)printf(" %08x",load(ctx.r6.u32+i));printf("' + NL + '");'}
    body = ('static const uint32_t C[][21]={' + ','.join('{' + ','.join('0x%08xu' % v for v in c) + '}' for c in cs) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            # the lamp at 0x1000: pos, colour, dir, +48 zero, +64 intensity, +68 cone, +72 penumbra
            'for(int i=0;i<12;i++)store(0x1000+4*i,c[i]);store(0x1040,c[12]);store(0x1044,c[13]);store(0x1048,c[14]);' +
            # the pass at 0x1200: handles = their offsets, +52 / +56 / +64
            'for(uint32_t h:{8u,12u,16u,20u,40u,44u,48u})store(0x1200+h,h);store(0x1200+52,c[15]);store(0x1200+56,c[16]);store(0x1200+64,c[17]);' +
            # the shader object at 0x1400: +12 -> 0x1500 (+20 group, +16 base)
            'store(0x1400+12,0x1500);' +
            # the masks the startup fills (as the AI harnesses do): xyz at 0x82860950, (0, 0, 0, 1) at 0x82860930
            'for(int i=0;i<3;i++)store(0x82860950+4*i,0xFFFFFFFFu);store(0x8286095C,0);for(int i=0;i<3;i++)store(0x82860930+4*i,0);store(0x8286093C,0x3F800000u);' +
            'printf("CASE");for(int i=0;i<21;i++)printf(" %08x",c[i]);printf("' + NL + '");' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1200;ctx.r4.u32=0x1400;ctx.r5.u32=0x1000;' +
            'float g[3];memcpy(g,&c[18],12);ctx.f1.f64=g[0];ctx.f2.f64=g[1];ctx.f3.f64=g[2];' +
            ('sub_%s(ctx,nullptr);}}' % fn) + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'cone_pass', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases', out.count('SET'), 'sets')


if __name__ == '__main__':
    main()
