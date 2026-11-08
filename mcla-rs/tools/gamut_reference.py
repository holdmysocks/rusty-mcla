"""The colour pickers' gamut limit run offline (generated code unchanged, scalar mode):

- F-85014: the excess of an RGB triple (r3 -> three floats): Y = .299 R + .587 G + .114 B, U = .492 (B - Y),
  V = .877 (R - Y) through the image's constants, C = |(U, V)|; f1 = Y + C when above 1.1, Y - C when below -0.1, else 0;
- F-52258: when that excess is not 0, the chroma scaled so Y + C or Y - C lands on the limit, and the triple written
  back (in place, r3).

The garage runs them on a paint swatch's colour (F-99653, SetVehicleColor) and on the picker's colour before
SaveCustomColor (F-53397) packs the ColorData word.

Cases: colours as the garage makes them (bytes x 1/255, the picker's hue / saturation / brightness corners), random
triples in 0 .. 1, out of range, negative, 0 / -0, subnormal, NaN.

Writes <out-dir>/gamut_ref.txt, one case a line, hex words: "r g b -> excess r' g' b'"."""
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
    fe, fl = ('sub_' + addr(a.tool, f) for f in ('F-85014', 'F-52258'))
    rnd = random.Random(52258)
    cases = []
    byte = lambda: rnd.randrange(256) * (1.0 / 255.0)
    for _ in range(3000):
        cases.append([byte(), byte(), byte()])
    for _ in range(2000):
        cases.append([rnd.uniform(0, 1) for _i in range(3)])
    corners = [0.0, 1.0, 0.5, 1e-40, -0.0]
    for r in corners:
        for g in corners:
            for b in corners:
                cases.append([r, g, b])
    for _ in range(2000):
        # (saturated: one channel 0, one 1, one between)
        t = [0.0, 1.0, rnd.uniform(0, 1)]
        rnd.shuffle(t)
        cases.append(t)
    odd = lambda: rnd.choice((rnd.uniform(-2, 3), -rnd.uniform(0, 1), rnd.uniform(1, 2), float('nan'), float('inf'), 1e-40, -1e-40))
    for _ in range(1000):
        cases.append([odd() if rnd.random() < 0.5 else rnd.uniform(0, 1) for _i in range(3)])
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = [fb(x) for c in cases for x in c]
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));' +
            'for(int i=0;i<3;i++)store(0x1000+4*i,D[3*c+i]);' +
            f'Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;{fe}(ctx,nullptr);' +
            'float e=float(ctx.f1.f64);uint32_t eb;memcpy(&eb,&e,4);' +
            f'Context k{{}};k.r1.u32=0x2F00;k.r3.u32=0x1000;{fl}(k,nullptr);' +
            'printf("%08x %08x %08x %08x' + NL + '",eb,load(0x1000),load(0x1004),load(0x1008));}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fe, fl], main_src, a.out_dir, 'gamut')
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'gamut_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
