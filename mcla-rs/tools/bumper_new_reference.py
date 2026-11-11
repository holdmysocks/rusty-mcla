"""A hanging bumper's set-up run offline: F-92886 (the movement byte, the two ends' rest points, the rest frame's copy)
on the generated code unchanged, its closing reset F-40944 stubbed (checked on its own, bumper_reset_hit_reference.py).
The caller (the car-anim binding) hands the point (0.5, 0, 0) - 0x82003A24 and 0x82000ED4 - and the slot's rest matrix.

Bumper at 0x1000 (filled with 0xCD first), the point at 0x2000, the matrix at 0x2100.

Writes <out-dir>/bumper_new_ref.txt: one case a line, "in -> out" (hex words): in = matrix(16) movement; out = bumper
+80 .. +108 (end A / B rest points, 8 words), +336 .. +396 (16), the byte +484 and whether the reset ran. Test
`damageparts::tests::bumper_new_matches_the_original`."""
import argparse
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)

fb, addr, mat = bs.fb, bs.addr, bs.mat


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn, reset = 'sub_' + addr(a.tool, 'F-92886'), 'sub_' + addr(a.tool, 'F-40944')
    rnd = random.Random(17)
    cases = []
    for k in range(a.cases):
        m = mat(rnd, rnd.choice((2.0, 50.0, 3000.0)))
        cases.append((m, rnd.choice((0, 1, 2, 3))))
    rows = ',\n'.join('{%s,%d}' % (','.join('0x%08x' % fb(float(x)) for x in m), mv) for m, mv in cases)
    nl = chr(10)
    setup = ('memset(objects,0,0x7000);for(int o=0;o<0x220;o+=4)store(0x1000+o,0xcdcdcdcd);store(0x6000,0);'
             'store(0x2000,0x3f000000);store(0x2004,0);store(0x2008,0);'
             'for(int k=0;k<16;k++)store(0x2100+4*k,c[k]);')
    outs = ('for(int o=80;o<112;o+=4)printf("%08x ",load(0x1000+o));'
            'for(int o=336;o<400;o+=4)printf("%08x ",load(0x1000+o));'
            'printf("%02x %d' + chr(92) + 'n",*pointer(0x1000+484),load(0x6000));')
    stubs = {reset: 'store(0x6000,1);'}
    body = ('static const uint32_t C[][17]={' + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x2000;ctx.r5.u32=c[16];ctx.r6.u32=0x2100;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);' + outs + '}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'bumpernew', stubs=stubs, vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%08x' % fb(float(x)) for x in m) + ' %d' % mv + ' -> ' + o for (m, mv), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'bumper_new_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
