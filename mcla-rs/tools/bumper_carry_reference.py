"""A teleport's carry of a hanging bumper run offline: F-87243 (its +400 copy of the car's matrix inverted by F-30106,
both ends' place and last place moved from that matrix to the new one) on the generated code unchanged.

Bumper at 0x1000 (the original's offsets): +112 / +128 end A place / last place, +192 / +208 end B, +400 the car matrix
copy. The new matrix at 0x2000.

Writes <out-dir>/bumper_carry_ref.txt: one case a line, "in -> out" (hex float words): in = old(16) new(16) posA prevA
posB prevB (4 each); out = bumper +112 .. +224 (28 words) and +400 .. +460 (16). Test
`damageparts::tests::bumper_carry_matches_the_original`."""
import argparse
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)

fb, addr, mat, xform = bs.fb, bs.addr, bs.mat, bs.xform


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-87243')
    rnd = random.Random(13)
    cases = []
    for k in range(a.cases):
        old, new = mat(rnd, 500.0), mat(rnd, 500.0)
        pts = []
        for e in range(4):
            c = xform(old, [rnd.uniform(-1, 1), rnd.uniform(-0.5, 1), rnd.uniform(-2.5, 2.5)])
            pts += c[:3] + [1.0]
        cases.append(old + new + pts)
    rows = ',\n'.join('{%s}' % ','.join('0x%08x' % fb(float(x)) for x in c) for c in cases)
    nl = chr(10)
    setup = ('memset(objects,0,0x7000);'
             'for(int k=0;k<4;k++){store(0x827D85D0+16*k+4*k,0x3f800000);store(0x827D8450+4*k,0x3f800000);store(0x827D8460+4*k,0x3f000000);store(0x828609E0+4*k,0x7f800000);'
             'store(0x82860950+4*k,k<3?0xffffffff:0);store(0x82860930+4*k,k<3?0:0x3f800000);}'
             'store(0x5800,0x5840);'
             'for(int k=0;k<16;k++){store(0x1000+400+4*k,c[k]);store(0x2000+4*k,c[16+k]);}'
             'for(int k=0;k<4;k++){store(0x1000+112+4*k,c[32+k]);store(0x1000+128+4*k,c[36+k]);store(0x1000+192+4*k,c[40+k]);store(0x1000+208+4*k,c[44+k]);}')
    outs = ('for(int o=112;o<224;o+=4)printf("%08x ",load(0x1000+o));'
            'for(int o=400;o<464;o+=4)printf("%08x ",load(0x1000+o));printf("' + chr(92) + 'n");')
    body = ('static const uint32_t C[][48]={' + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x2000;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);' + outs + '}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'bumpercarry', vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%08x' % fb(float(x)) for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'bumper_carry_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
