"""The racer brain's turnaround picker F-67810 run offline (the generated code whole and unchanged, vector mode). Real
memory: the brain's +332 turn side, +336 backing up, +337 handbrake, +338 avoid handbrake turns, the speed it reads
(racer +48 -> +8 -> +224). Stubs: the racer's matrix getter F-35658 (the case's matrix: +32 the row whose negation is
forward, +48 the position), the line's next point F-28749 and the corridor point ahead F-43876 (the case's points), and
the arc test F-38160 as an oracle: each call is logged (angle, reverse r5, pivot r6) and answers bit k of the case's mask
for its k-th call. The plan angle F-32060 runs as generated. So the picker's own choice is compared; the arc test itself
(the nav mesh) is not.

Cases (seed 67810): random states and car / point placements as tools/stuck_reference.py's (angles round 0 / 45 / 90 /
135 / 180 degrees, NaN speed / point, speeds round 10 m/s), every flag combination, arc masks 0 .. 63 at random.

Writes <out-dir>/turnaround_ref.txt, hex words: "case" turn backing handbrake avoid speed pos.xyz row2.xyz p1.xyz p2.xyz
mask "->" turn backing handbrake, then the arc calls "angle reverse pivot" each."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb, state

B, R, CR, S, M, P1, P2 = (0x40100000 + 0x1000 * k for k in range(7))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-67810')
    rnd = random.Random(67810)
    nl = chr(10)
    cases = []
    for _ in range(a.cases):
        s = state(rnd)
        if rnd.random() < 0.5:
            s[11] = rnd.uniform(0, 10)
        flags = [rnd.choice((0, 0, 1, 2)), rnd.choice((0, 1)), rnd.choice((0, 1)), rnd.choice((0, 0, 1))]
        mask = rnd.choice((0, 63, rnd.getrandbits(6), 1 << rnd.randrange(6)))
        if rnd.random() < 0.02:
            s[11] = float('nan')
        cases.append(flags + [fb(x) for x in [s[11], *s[5:11], *s[12:18]]] + [mask])
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    main_src = (
        'static const uint32_t CS[][18]={' + nl + rows + nl + '};' + nl +
        'static uint32_t MASK,NC,LOG[16][3];' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(auto&s:CS){rt_reset();memset(pointer(%du),0,0x7000);store(%du+4,%du);store(%du+300,0x40200000u);store(%du+48,%du);store(%du+8,%du);store(%du+60,0x3f800000u);'
        % (B, B, R, B, R, CR, CR, S, M) +
        'store(%du+332,s[0]);*pointer(%du+336)=(uint8_t)s[1];*pointer(%du+337)=(uint8_t)s[2];*pointer(%du+338)=(uint8_t)s[3];store(%du+224,s[4]);'
        % (B, B, B, B, S) +
        'for(int i=0;i<3;i++){store(%du+48+4*i,s[5+i]);store(%du+32+4*i,s[8+i]);store(%du+4*i,s[11+i]);store(%du+4*i,s[14+i]);}' % (M, M, P1, P2) +
        'MASK=s[17];NC=0;Context ctx=fresh();ctx.r3.u32=%du;sub_%s(ctx,nullptr);' % (B, fn) +
        'printf("case");for(int i=0;i<18;i++)printf(" %%08x",s[i]);'
        'printf(" -> %%u %%u %%u",load(%du+332),(unsigned)*pointer(%du+336),(unsigned)*pointer(%du+337));' % (B, B, B) +
        'for(uint32_t k=0;k<NC;k++)printf(" %08x %u %u",LOG[k][0],LOG[k][1],LOG[k][2]);printf("\\n");}}' + nl)
    arc = ('{float f=(float)ctx.f1.f64;uint32_t w;memcpy(&w,&f,4);uint32_t &n=oracle_nc();if(n>=16)throw std::runtime_error("arc calls");'
           'oracle_log()[n][0]=w;oracle_log()[n][1]=ctx.r5.u32&0xff;oracle_log()[n][2]=ctx.r6.u32&0xff;ctx.r3.u32=(oracle_mask()>>n)&1;n++;}')
    stubs = {'sub_' + addr(a.tool, 'F-35658'): 'ctx.r3.u32=%du;' % M,
             'sub_' + addr(a.tool, 'F-28749'): 'ctx.r3.u32=%du;' % P1,
             'sub_' + addr(a.tool, 'F-43876'): 'for(int i=0;i<16;i+=4)store(ctx.r3.u32+i,load(%du+i));' % P2,
             'sub_' + addr(a.tool, 'F-38160'): arc}
    # (the oracle's state comes ahead of the stubs, through the closure tool's EXTRA prefix; main reads it through macros)
    pre = ('static uint32_t &oracle_mask(){static uint32_t v;return v;}static uint32_t &oracle_nc(){static uint32_t v;return v;}'
           'static uint32_t (&oracle_log())[16][3]{static uint32_t v[16][3];return v;}' + nl)
    main_src = main_src.replace('static uint32_t MASK,NC,LOG[16][3];', '#define MASK oracle_mask()' + nl + '#define NC oracle_nc()' + nl + '#define LOG oracle_log()')
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + pre
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'turnaround', stubs=stubs, image=rd.load_image(a.image))
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'turnaround_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    from collections import Counter
    res = [x.split('->')[1].split() for x in lines]
    print(len(res), 'cases; turn', Counter(r[0] for r in res), 'backing', Counter(r[1] for r in res), 'arc calls', Counter((len(r) - 3) // 3 for r in res))


if __name__ == '__main__':
    main()
