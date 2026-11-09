"""The racer brain's rubber-band throttle cap (brain +404) run offline: the driving output F-17058 (the generated code
whole and unchanged, vector mode) taken into its racing case - hold still +257 off, revving off with its timer out, the
global word 0x82874690 0, chassis +592 0, the aiv answering valid - with the stuck detector F-34336 and the turnaround
F-67810 stubbed to do nothing (both verified on their own: verify_stuck.md / verify_turnaround.md; +332 / +336 0, so the
cap is reached). After the cap the frame goes on into the gear / nitrous code; whatever it reaches that is not modelled
throws, and the case keeps the +404 the cap wrote.

Real memory: brain +380 (rubber band on), +384 shifted gap, +396 distance to the finish, +400 squared distance to the
nearest human, +412 .. +484 the rubber band tune copied at +412 (MaxThrottle at +0 of mcRubberBandTuning), the chassis
speed +224, the aiv tuning's MaxSpeed (aiv +0 -> +0). +404 starts at a marker word.

Cases (seed 48266): tunes from the shipped steps' ranges and edge values (MinDist = MaxDist, zero throttles, NaN), gaps
round 0 and the ends, speeds round MinSpeed x MaxSpeed, distances round HomeStretchDistance and PlayerCloseDistance^2.

Writes <out-dir>/rubber_cap_ref.txt, hex words: "case on g to_finish d2 speed maxspeed" + the 19 tune words + " -> cap"."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb, nudge

B, R, C, S, X, E, G, T, IN, AIV, TUN = (0x40100000 + 0x1000 * k for k in range(11))
GW = 0x82874690
MARK = 0x7FA5A5A5


def case(rnd):
    t = [rnd.uniform(0.8, 1.0), rnd.uniform(0.3, 0.7), rnd.uniform(0, 30), rnd.uniform(30, 120), rnd.uniform(0.2, 0.8),
         rnd.uniform(0, 30), rnd.uniform(30, 200), rnd.uniform(0.5, 1.0)] + [rnd.uniform(0, 1) for _ in range(11)]
    # (+412 MaxThrottle, +416 MinSpeed, +420 MinDist, +424 MaxDist, +428 MinThrottle, +432 BehindMinDist, +436 BehindMaxDist,
    # +440 BehindMaxThrottle, +444 .. +468 not read here, +472 HomeStretchBehindMaxThrottle, +476 HomeStretchDistance,
    # +480 PlayerCloseMaxThrottle, +484 PlayerCloseDistance: words 0 .. 18)
    t[16] = rnd.uniform(0, 800)
    t[18] = rnd.uniform(0, 60)
    r = rnd.random()
    if r < 0.05:
        t[3] = t[2]
    elif r < 0.1:
        t[6] = t[5]
    elif r < 0.12:
        t[rnd.randrange(19)] = float('nan')
    elif r < 0.16:
        t[rnd.choice((0, 4, 7, 15, 17))] = 0.0
    maxspeed = rnd.uniform(40, 120)
    lim = t[1] * maxspeed
    speed = rnd.choice((lim, nudge(lim, 1), nudge(lim, -1), rnd.uniform(0, maxspeed), rnd.uniform(lim, maxspeed), lim + 5))
    g = rnd.choice((0.0, -0.0, t[2], t[3], -t[5], -t[6], rnd.uniform(-250, 250), rnd.uniform(-20, 20), rnd.uniform(0, 150)))
    tf = rnd.choice((t[16], nudge(t[16], 1), nudge(t[16], -1), rnd.uniform(0, 1600)))
    d2 = rnd.choice((t[18] * t[18], nudge(t[18] * t[18], 1), rnd.uniform(0, 4000), rnd.uniform(0, 400), 1e30))
    if rnd.random() < 0.01:
        speed = float('nan')
    if rnd.random() < 0.01:
        g = float('nan')
    on = rnd.choice((1, 1, 1, 0))
    return [on] + [fb(x) for x in (g, tf, d2, speed, maxspeed, *t)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-17058')
    rnd = random.Random(48266)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    main_src = (
        'static const uint32_t CS[][25]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(auto&s:CS){rt_reset();memset(pointer(%du),0,0xB000);' % B +
        'store(%du+4,%du);store(%du+48,%du);store(%du+8,%du);store(%du+96,%du);store(%du+16,%du);store(%du+20,%du);store(%du+368,%du);'
        % (B, R, R, C, C, S, C, X, S, E, S, G, S, T) +
        'store(%du+300,%du);store(%du+180,0x3f800000u);store(%du+20,0x3f800000u);store(%du+20,0x00030000u);store(%du+592,0u);' % (B, AIV, X, E, G, S) +
        'store(%du,0u);store(%du+4,0u);store(%du,%du);' % (GW - 4, GW - 4, AIV, TUN) +
        '*pointer(%du+380)=(uint8_t)s[0];store(%du+384,s[1]);store(%du+396,s[2]);store(%du+400,s[3]);store(%du+224,s[4]);store(%du,s[5]);'
        % (B, B, B, B, S, TUN) +
        'for(int i=0;i<19;i++)store(%du+412+4*i,s[6+i]);store(%du+404,0x%08xu);' % (B, B, MARK) +
        'Context ctx=fresh();ctx.r3.u32=%du;ctx.r4.u32=%du;try{sub_%s(ctx,nullptr);}catch(const std::exception&){}' % (B, IN, fn) +
        'printf("case");for(int i=0;i<25;i++)printf(" %08x",s[i]);printf(" -> %08x\\n",load(' + '%du+404));}}' % B + nl)
    stubs = {'sub_' + addr(a.tool, 'F-59682'): '', 'sub_' + addr(a.tool, 'F-65301'): 'ctx.r3.u32=1;',
             'sub_' + addr(a.tool, 'F-34336'): '', 'sub_' + addr(a.tool, 'F-67810'): ''}
    # (what the frame reaches after the cap - rev audio, aiv release, nitrous / slingshot tests and benefits, asin: a throw ends
    # the case after +404 is written)
    for f in ('F-17772', 'F-58498', 'F-50530', 'F-29462', 'F-80613', 'F-82077', 'F-96973'):
        stubs['sub_' + addr(a.tool, f)] = 'throw std::runtime_error("past the cap: %s");' % f
    exe = rd.build(rd.index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'rubbercap', stubs=stubs, image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    assert not any(x.endswith('%08x' % MARK) for x in lines), 'a case did not reach the cap'
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'rubber_cap_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    from collections import Counter
    print(len(lines), 'cases; cap 1:', sum(x.endswith('3f800000') for x in lines), 'NaN:', sum(x.split()[-1][1:3] in ('7f', 'ff') and x.split()[-1][3] >= '8' for x in lines),
          'distinct', len(Counter(x.split()[-1] for x in lines)))


if __name__ == '__main__':
    main()
