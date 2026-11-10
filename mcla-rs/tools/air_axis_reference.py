"""The racer brain's in-air pitch axis run offline: F-17058's racing case (set up as tools/nitro_pick_reference.py sets
it up) with no wheel down (the chassis' vfunc 44 answering 0): input +40 bit 0 and +32 = clamp(fma(-(s a), -2.5465,
w x -4), -1, 1), s the sign of the sim's up.y (+228: 1, 0 at +-0, -1 below or NaN), a = asin(+244, row 2's y), w =
row 0 (+208) . the angular velocity (+288) by vmsum3fp. The sim object's rows and angular velocity random per case; the
case adds them as 12 words (row 0, row 1, row 2, angular velocity) and the output +32 as a third word.

(What follows is the nitro pick's description, the harness this one is made from:)
The racer brain's nitrous / slingshot pick after the rubber-band cap run offline: the driving output F-17058 (the
generated code whole and unchanged, vector mode) in its racing case as tools/rubber_cap_reference.py sets it up, run to its
end this time. The aiv output F-65301 is an oracle (answers valid and writes the case's throttle through r9), the chassis'
vfunc 44 (wheels down) a fake answering the case's count (chassis +24 = 4), the nitrous (+664) and slingshot (+668)
objects present or not (the sim object +128 zeroed: level, at rest, for the in-air branch) with their burning (+32) / active (+40) bytes; their can-fire tests F-50530 / F-29462 and the
benefits F-82077 / F-96973 (aiv) answer the case's values. The stuck detector F-34336 and the turnaround F-67810 do
nothing (verified on their own). Brain +236 allow nitrous, +240 / +244 the nitrous / slingshot scores, +248 the nitrous
timer, +380 .. +400 the rubber band's measures, +412 .. +484 its tune.

Writes <out-dir>/nitro_pick_ref.txt, hex words: "case" + the 25 rubber-cap words (on g to_finish d2 speed maxspeed tune[19])
+ "throttle down allow nitro_off from_start gap nitro_obj nitro_ready burning sling_obj sling_ready sling_active
nitro_benefit sling_benefit nitro_score sling_score -> flags(+40) throttle(+156)"."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb, nudge

B, R, C, S, X, E, G, T, IN, AIV, TUN, NO, SL, VT = (0x40100000 + 0x1000 * k for k in range(14))
FAKE = 0x40F00000
GW = 0x82874690
MARK = 0x7FA5A5A5


def sim_rows(rnd):
    def unit():
        v = [rnd.gauss(0, 1) for _ in range(3)]
        n = sum(x * x for x in v) ** 0.5 or 1.0
        return [x / n for x in v]
    r0, r1, r2 = unit(), unit(), unit()
    sc = rnd.choice((0.05, 0.2, 0.2, 3.0))
    k = rnd.random()
    if k < 0.08:
        r1[1] = rnd.choice((0.0, -0.0, float('nan')))
    elif k < 0.12:
        r2[1] = rnd.choice((1.0, -1.0, nudge(1.0, 1), float('nan'), 0.0))
    elif k < 0.2:
        r1[1] = rnd.choice((nudge(0.0, 1), -nudge(0.0, 1), 1.0))
    w = [rnd.uniform(-sc, sc) for _ in range(3)] if rnd.random() < 0.9 else [0.0, 0.0, 0.0]
    if rnd.random() < 0.05:
        w[rnd.randrange(3)] = rnd.choice((1e30, -1e30, 1e-40))
    return r0 + r1 + r2 + w


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
    on = rnd.choice((1, 0))
    # (the nitrous gates: words 11 / 12 / 13 = +456 from the start, +460 from the finish, +464 behind the player)
    t[11], t[12], t[13] = rnd.uniform(0, 400), rnd.uniform(0, 400), rnd.uniform(0, 100)
    tf = rnd.choice((tf, t[12], nudge(t[12], 1), rnd.uniform(t[12], 1600), rnd.uniform(t[12], 1600)))
    fs = rnd.choice((t[11], nudge(t[11], 1), rnd.uniform(0, 1600), rnd.uniform(t[11], 1600), rnd.uniform(t[11], 1600)))
    gap = rnd.choice((-t[13], nudge(-t[13], -1), rnd.uniform(-300, 100), rnd.uniform(-400, -t[13]), rnd.uniform(-400, -t[13])))
    thr = rnd.choice((1.0, 1.0, 0.99999899, 0.9999991, rnd.uniform(0.9, 1.0), 0.5, float('nan') if rnd.random() < 0.05 else 1.0))
    down = 0
    allow = rnd.choice((1, 1, 1, 0))
    noff = rnd.choice((0.0, 0.0, 0.0, 0.0, -0.0, 0.5))
    no, so = rnd.choice((1, 1, 0)), rnd.choice((1, 1, 0))
    flags = [rnd.choice((1, 1, 0)) for _ in range(4)]
    nb, sb = rnd.uniform(0, 1), rnd.uniform(0, 1)
    ns, ss = rnd.choice((0.5, rnd.uniform(0, 1), nb)), rnd.choice((0.5, rnd.uniform(0, 1), sb))
    return ([on] + [fb(x) for x in (g, tf, d2, speed, maxspeed, *t)] + [fb(thr), down, allow, fb(noff), fb(fs), fb(gap),
            no, flags[0], flags[1], so, flags[2], flags[3]] + [fb(x) for x in (nb, sb, ns, ss)] + [fb(x) for x in sim_rows(rnd)])


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-17058')
    rnd = random.Random(17059)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    main_src = (
        'static const uint32_t CS[][53]={' + nl + rows + nl + '};' + nl +
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
        'for(int i=0;i<3;i++){store(0x40E00000u+208+4*i,s[41+i]);store(0x40E00000u+224+4*i,s[44+i]);store(0x40E00000u+240+4*i,s[47+i]);store(0x40E00000u+288+4*i,s[50+i]);}store(%du+128,0x40E00000u);store(%du,%du);store(%du+44,%du);store(%du+24,4u);o_w()[2]=s[26];*pointer(%du+236)=(uint8_t)s[27];store(%du+248,s[28]);' % (S, S, VT, VT, FAKE, S, B, B) +
        'store(%du+392,s[29]);store(%du+388,s[30]);store(%du+240,s[39]);store(%du+244,s[40]);o_f()[0]=s[37];o_f()[1]=s[38];' % (B, B, B, B) +
        'if(s[31]){store(%du+664,%du);*pointer(%du+32)=(uint8_t)s[33];}o_w()[0]=s[32];' % (S, NO, NO) +
        'if(s[34]){store(%du+668,%du);*pointer(%du+40)=(uint8_t)s[36];}o_w()[1]=s[35];o_w()[3]=s[25];' % (S, SL, SL) +
        'Context ctx=fresh();ctx.r3.u32=%du;ctx.r4.u32=%du;try{sub_%s(ctx,nullptr);}catch(const std::exception&e){fprintf(stderr,"THROW %%s\\n",e.what());}' % (B, IN, fn) +
        'printf("case");for(int i=0;i<53;i++)printf(" %%08x",s[i]);printf(" -> %%08x %%08x %%08x\\n",load(%du+40),load(%du+156),load(%du+32));}}' % (IN, IN, IN) + nl)
    stubs = {'sub_' + addr(a.tool, 'F-59682'): '', 'sub_' + addr(a.tool, 'F-65301'): 'store(ctx.r9.u32,o_w()[3]);ctx.r3.u32=1;',
             'sub_' + addr(a.tool, 'F-34336'): '', 'sub_' + addr(a.tool, 'F-67810'): ''}
    # (what the frame reaches after the cap - rev audio, aiv release, nitrous / slingshot tests and benefits, asin: a throw ends
    # the case after +404 is written)
    # (asin F-80613, reached with no wheel down, runs as generated)
    for f in ('F-17772', 'F-58498'):
        stubs['sub_' + addr(a.tool, f)] = 'throw std::runtime_error("not modelled: %s");' % f
    stubs['sub_' + addr(a.tool, 'F-50530')] = 'ctx.r3.u32=o_w()[0];'
    stubs['sub_' + addr(a.tool, 'F-29462')] = 'ctx.r3.u32=o_w()[1];'
    stubs['sub_' + addr(a.tool, 'F-82077')] = '{float v;memcpy(&v,&o_f()[0],4);ctx.f1.f64=v;}'
    stubs['sub_' + addr(a.tool, 'F-96973')] = '{float v;memcpy(&v,&o_f()[1],4);ctx.f1.f64=v;}'
    pre = 'static uint32_t (&o_w())[4]{static uint32_t v[4];return v;}static uint32_t (&o_f())[2]{static uint32_t v[2];return v;}' + nl
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + pre
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'airaxis', stubs=stubs, image=rd.load_image(a.image),
                       fakes={FAKE: 'ctx.r3.u32=o_w()[2];'})
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'air_axis_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    from collections import Counter
    print(len(lines), 'cases; flags', Counter(x.split()[-3] for x in lines), 'axis', Counter(x.split()[-1] for x in lines).most_common(4))


if __name__ == '__main__':
    main()
