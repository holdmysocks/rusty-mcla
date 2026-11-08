"""The fragment type's mass F-25823 run offline (generated code unchanged, scalar mode): the children (count byte +303,
pointer array +212) summed from 0.0 with fadds one child after another (the loop unrolled by four, then the rest),
each child's mass at its +4.

Cases: counts 0 .. 64 and the byte's top 255, masses as shipped (0.1 .. 400), zeros and -0, mixed magnitudes (the
order of the additions shows), inf, NaN.

Writes <out-dir>/entry_mass_ref.txt, one case a line, hex: "n m0 m1 .. -> sum"."""
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
    fn = 'sub_' + addr(a.tool, 'F-25823')
    rnd = random.Random(25823)
    cases = []
    for i in range(3000):
        n = rnd.choice((0, 1, 2, 3, 4, 5, 7, 8, 9, rnd.randrange(0, 65), 255 if i % 300 == 0 else 13))
        ms = []
        for _k in range(n):
            ms.append(rnd.choice((rnd.uniform(0.1, 400), rnd.uniform(0.1, 5), 0.0, -0.0, rnd.uniform(1e-4, 1e-2), rnd.uniform(1e5, 1e7),
                                  float('inf') if rnd.random() < 0.01 else 1.0, float('nan') if rnd.random() < 0.01 else 2.5)))
        cases.append([n] + [fb(m) for m in ms])
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = [x for c in cases for x in c]
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'size_t at=0;for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));uint32_t n=D[at++];' +
            'store(0x1000+300,n);store(0x1000+212,0x1400);' +
            'for(uint32_t k=0;k<n;k++){store(0x1400+4*k,0x1800+8*k);store(0x1800+8*k+4,D[at++]);}' +
            f'Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;{fn}(ctx,nullptr);' +
            'float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);printf("%08x' + NL + '",u);}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fn], main_src, a.out_dir, 'entry_mass')
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'entry_mass_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
