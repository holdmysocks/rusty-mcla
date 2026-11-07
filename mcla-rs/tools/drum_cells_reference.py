"""The cockpit drum readout's cells run offline: F-27249 (a drum value to its cell of the odometer font, out of range 63,
past the count 0) on the generated code unchanged. Its matrix call F-60960 is a no-op stub; the draw that follows the
remap starts with F-94845, replaced by a stub that prints the 16 cells the remap left on the stack (r1 + 160) and
returns 0, so nothing is drawn. The values (r4) sit at 0x1000, the matrix (r6) at 0x1100, the object (r3) at 0x1200.

Writes <out-dir>/drum_cells_ref.txt, one case a line: "count v0 .. v15 -> c0 .. c15" (hex float bits).
Test `readouts::tests::drum_cells_match_the_original`."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
from recomp_closure import index, build, run


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
    fc, fm, fd = addr(a.tool, 'F-27249'), addr(a.tool, 'F-60960'), addr(a.tool, 'F-94845')
    rnd = random.Random(23)
    edges = [0.0, -0.0, 1e-7, -1e-7, 0.5, 7.0, 7.9999995, 8.0, 8.5, 9.0, 9.9999995, 10.0, 10.5, 16.0, 24.0, 63.0, -1.0,
             -63.0, 1e30, -1e30, float('inf'), float('-inf'), float('nan')]
    cases = []
    for n in range(0, 17):
        cases.append((n, [edges[(n + k) % len(edges)] for k in range(16)]))
    for _ in range(2000):
        vals = [rnd.choice((rnd.uniform(0, 10), rnd.uniform(0, 10), rnd.randrange(10) + rnd.choice((0.0, 0.9, 0.95, 0.999)),
                            rnd.uniform(-2, 12), rnd.choice(edges))) for _ in range(16)]
        cases.append((rnd.choice((1, 2, 4, 6, 6, 6, 8, 15, 16)), vals))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%d,%s},' % (n, ','.join('0x%08xu' % fb(v) for v in vals)) for n, vals in cases)
    stub = ('for(int k=0;k<16;k++)printf(k?" %08x":"%08x",load(ctx.r1.u32+160+4*k));printf("' + NL + '");ctx.r3.u64=0;')
    body = ('static const uint32_t C[][17]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));for(int k=0;k<16;k++)store(0x1000+4*k,c[1+k]);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1200;ctx.r4.u32=0x1000;ctx.r5.u64=c[0];ctx.r6.u32=0x1100;' +
            'ctx.r7.u64=0;ctx.r8.u64=0;ctx.r9.u64=0;ctx.r10.u32=0x1300;' +
            ('sub_%s(ctx,nullptr);}' % fc) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    # (the draw's other calls, never reached once F-94845 returns 0: empty stubs, so the closure stays the remap)
    rest = ('F-56814', 'F-17339', 'F-89493', 'F-61918', 'F-24298', 'F-91850', 'F-55601', 'F-64407', 'F-90259', 'F-46773')
    stubs = {'sub_' + fm: '', 'sub_' + fd: stub}
    stubs.update({'sub_' + addr(a.tool, f): '' for f in rest})
    exe = build(idx, ['sub_' + fc], main_src, a.out_dir, 'cells', stubs=stubs, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%d %s -> %s' % (n, ' '.join('%08x' % fb(v) for v in vals), o) for (n, vals), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'drum_cells_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
