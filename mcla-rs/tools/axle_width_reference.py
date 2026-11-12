"""The post-build set-up's width terms (F-67116, run once by F-25334) offline on the generated code unchanged: for every
axle of the chassis's list +44 F-91839 writes the axle's +96 = fma(|+100|, f1, 1.0), for every vehSuspension of the
list +52 F-12544 writes its +88 = fma(+92, f1, 1.0); f1 = 0.2 - the wheel's width (+424), less (FenderEdge stock - wide)
of mcCarModelTune ([[chassis +4] +76] +96: +88 / +92 when the wheel's rest z (+392) is below 0.0, else +96 / +100) where
the set-up's wide body byte ([[chassis +4] +132] +2203) is not 0. Nothing is done for a chassis with two wheels (+24).

Memory: chassis 0x1000 (+4 = car 0x1400; car +76 = model 0x1600 (+96 = tune 0x1700), +132 = cfg 0x1800), axles 0x2000 +
0x100 k (+84 their left wheel 0x3000 + 0x100 k), parts 0x4000 + 0x100 k (+80 their wheel 0x5000 + 0x100 k), lists at
0x6000 / 0x6100.

Writes <out-dir>/axle_width_ref.txt: one case a line, "in -> out" (hex words): in = wheels wide naxles nparts, then
fender stock/wide front, stock/wide rear, then per axle a100 width rest_z and per part inv width rest_z; out = per axle
+96, per part +88. Test `susp_parts::tests::width_terms_match_the_original`."""
import argparse
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs


fb, addr = bs.fb, bs.addr


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    a = ap.parse_args()
    setup, = ('sub_' + addr(a.tool, k) for k in ('F-67116',))
    rnd = random.Random(31)
    cases = []
    for k in range(a.cases):
        wheels = rnd.choice((4, 4, 4, 2))
        wide = rnd.choice((0, 0, 1, 2))
        na, np_ = rnd.randint(0, 2), rnd.randint(0, 4)
        fe = [rnd.choice((0.0, rnd.uniform(0.8, 1.1))) for _ in range(4)]
        ax = [[rnd.choice((0.0, rnd.uniform(-1.5, 1.5))), 0.195 + 0.01 * rnd.randint(0, 16), rnd.choice((0.0, -0.0, rnd.uniform(-1.6, 1.6)))] for _ in range(na)]
        ps = [[rnd.uniform(-3, 3), 0.195 + 0.01 * rnd.randint(0, 16), rnd.choice((0.0, -0.0, rnd.uniform(-1.6, 1.6)))] for _ in range(np_)]
        cases.append(([wheels, wide, na, np_], fe + sum(ax, []) + sum(ps, [])))
    nl = chr(10)
    rows = ',\n'.join('{%s}' % ','.join(['%d' % x for x in ints] + ['0x%08x' % fb(float(x)) for x in f] + ['0'] * (4 + 12 + 12 - len(ints) - len(f))) for ints, f in cases)
    body = ('static const uint32_t C[][28]={' + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;memset(objects,0,0x7000);'
            'store(0x1000+24,c[0]);store(0x1000+4,0x1400);store(0x1400+76,0x1600);store(0x1600+96,0x1700);store(0x1400+132,0x1800);'
            'store(0x1800+2200,c[1]&0xff);'
            'for(int k=0;k<4;k++)store(0x1700+88+4*k,c[4+k]);'
            'store(0x1000+40,c[2]);store(0x1000+44,0x6000);store(0x1000+48,c[3]);store(0x1000+52,0x6100);'
            'int q=8;for(int k=0;k<(int)c[2];k++){store(0x6000+4*k,0x2000+0x100*k);store(0x2000+0x100*k+84,0x3000+0x100*k);'
            'store(0x2000+0x100*k+100,c[q]);store(0x3000+0x100*k+424,c[q+1]);store(0x3000+0x100*k+392,c[q+2]);q+=3;}'
            'for(int k=0;k<(int)c[3];k++){store(0x6100+4*k,0x4000+0x100*k);store(0x4000+0x100*k+80,0x5000+0x100*k);'
            'store(0x4000+0x100*k+92,c[q]);store(0x5000+0x100*k+424,c[q+1]);store(0x5000+0x100*k+392,c[q+2]);q+=3;}'
            '{Context ctx{};ctx.r3.u32=0x1000;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + setup + '(ctx,nullptr);}'
            'printf("x");for(int k=0;k<(int)c[2];k++)printf(" %08x",load(0x2000+0x100*k+96));'
            'for(int k=0;k<(int)c[3];k++)printf(" %08x",load(0x4000+0x100*k+88));printf("' + chr(92) + 'n");}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [setup], main_src, a.out_dir, 'axle_width')
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%d' % x for x in ints) + ' ' + ' '.join('%08x' % fb(float(x)) for x in f) + ' -> ' + o[1:].strip() for (ints, f), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'axle_width_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
