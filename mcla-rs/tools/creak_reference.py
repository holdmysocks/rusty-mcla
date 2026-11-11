"""The bumper creak's speed getter run offline: F-57399 (the fastest moving bumper end relative to the car, and where it
is) on the generated code unchanged. A bumper at 0x1000 (the original's offsets, DAMAGE_PARTS_RULE.md 2.1 / 3): +0 the
part (0 or not), +512 / +516 the ends' states, end A's car-space place +160 and its last +144, world place +112; end B's
+240 / +224 / +192. The frame block's +12 (1 / dt) = 29.999998, as running (F-75502 writes 1.0 / dt).

Writes <out-dir>/creak_ref.txt: one case a line, "in -> out" (hex words): in = part stateA stateB (ints), A now(3) A last(3)
A world(4) B now(3) B last(3) B world(4) (floats); out = f1 and the four words at the place pointer (0xDEADBEEF where it
was not written). Test `damageparts::tests::creak_matches_the_original`."""
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
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-57399')
    rnd = random.Random(13)
    cases = []

    def vec(s):
        return [rnd.uniform(-s, s) for _ in range(3)]
    for k in range(a.cases):
        part = 0 if rnd.random() < 0.05 else 1
        sa, sb = rnd.choice((0, 1, 2)), rnd.choice((0, 1, 2))
        ends = []
        for _ in range(2):
            now = vec(1.0)
            move = rnd.choice((0.0, 0.001, 0.01, 0.05))
            last = now if rnd.random() < 0.1 else [x + rnd.uniform(-move, move) for x in now]
            ends += now + last + vec(50.0) + [1.0]
        if rnd.random() < 0.05:
            # (equal speeds: the tie goes to B)
            ends[10:13] = ends[0:3]
            ends[13:16] = ends[3:6]
        cases.append(([part, sa, sb], ends))
    rows = ',\n'.join('{%s}' % ','.join(['%d' % x for x in ints] + ['0x%08x' % fb(float(x)) for x in f]) for ints, f in cases)
    n = 3 + 20
    nl = chr(10)
    setup = ('memset(objects,0,0x7000);store(0x827D750C,0x41efffff);store(0x1000,c[0]?0x2000:0);store(0x1000+512,c[1]);store(0x1000+516,c[2]);'
             'for(int k=0;k<3;k++){store(0x1000+160+4*k,c[3+k]);store(0x1000+144+4*k,c[6+k]);'
             'store(0x1000+240+4*k,c[13+k]);store(0x1000+224+4*k,c[16+k]);}'
             'for(int k=0;k<4;k++){store(0x1000+112+4*k,c[9+k]);store(0x1000+192+4*k,c[19+k]);store(0x6000+4*k,0xdeadbeef);}')
    outs = ('{float t=float(ctx.f1.f64);uint32_t u;memcpy(&u,&t,4);printf("%08x",u);}'
            'for(int o=0;o<16;o+=4)printf(" %08x",load(0x6000+o));printf("' + chr(92) + 'n");')
    body = ('static const uint32_t C[][%d]={' % n + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x6000;ctx.f1.f64=12345.0;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);' + outs + '}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'creak', vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%d' % x for x in ints) + ' ' + ' '.join('%08x' % fb(float(x)) for x in f) + ' -> ' + o for (ints, f), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'creak_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
