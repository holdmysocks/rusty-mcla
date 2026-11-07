"""The cockpit drum readout's digits run offline: F-17778 (the value x 10, the fraction, the tenths' share, one cell a digit
with the roll in the last tenth) on the generated code unchanged, its draw F-27249 replaced by a stub that prints the 16
values F-17778 hands it (r4) and the count (r5). The object (r3) is zero memory at 0x1000; the font index (r7) is 0.

Writes <out-dir>/drum_values_ref.txt, one case a line: "value count tenths -> count v0 .. v15" (hex float bits).
Test `readouts::tests::drum_values_match_the_original`."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
from recomp_closure import index, build, run

# (the loop counter: r2 is free in the shim's context; fctiwz's clamp needs simde's load_sd - the vector shim)
recomp_closure.EXTRA += '#define ctr r2\n'


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
    fv, fd = addr(a.tool, 'F-17778'), addr(a.tool, 'F-27249')
    rnd = random.Random(17)
    cases = []
    # (odometer miles, trip tenths, edges of the roll, rounding near .9 / .99, zero and negatives)
    for v in (0.0, 0.05, 0.09, 0.1, 0.9, 0.95, 0.99, 0.999, 1.0, 9.9, 9.95, 9.99, 10.0, 99.95, 99.99, 123.456, 999999.0,
              999999.9, 1234567.0, -0.5, -12.3, 0.0999, 19.91, 199.96, 5.0):
        for n in (1, 3, 6, 15):
            for t in (0, 1):
                cases.append((v, n, t))
    for _ in range(3000):
        v = rnd.choice((rnd.uniform(0, 10), rnd.uniform(0, 1000), rnd.uniform(0, 1e6), round(rnd.uniform(0, 500), 1) + rnd.choice((0, 0.05, 0.09, -0.01))))
        cases.append((v, rnd.choice((1, 2, 4, 6, 6, 6, 8, 15)), rnd.randrange(2)))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{0x%08x,%d,%d},' % (fb(v), n, t) for v, n, t in cases)
    stub = ('printf("%08x",ctx.r5.u32);for(int k=0;k<16;k++)printf(" %08x",load(ctx.r4.u32+4*k));printf("' + NL + '");')
    body = ('static const uint32_t C[][3]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            'ctx.f1.f64=(double)fl(c[0]);ctx.f2.f64=1.0;ctx.r5.u64=c[1];ctx.r6.u64=c[2];ctx.r7.u64=0;ctx.r8.u64=0;ctx.r9.u64=0;ctx.r10.u64=1;' +
            ('sub_%s(ctx,nullptr);}' % fv) + nl)
    main_src = ('static float fl(uint32_t u){float f;memcpy(&f,&u,4);return f;}' + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fv], main_src, a.out_dir, 'drum', stubs={'sub_' + fd: stub}, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%08x %d %d -> %s' % (fb(v), n, t, o) for (v, n, t), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'drum_values_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
