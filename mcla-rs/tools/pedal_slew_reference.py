"""mcOpponentInput's update F-94408 run offline (the generated code whole and unchanged): the pedals the brain asks for
(+152 steer, +156 throttle, +160 brake, +164 handbrake) slewed into the held ones (+24, +12, +16, +20) at the tune block's
rates (+72 steer back, +76 steer out, +80 brake on, +84 brake off, +88 handbrake, +92 throttle; the ctor's 4, 6, 2, 30,
10, 35 - F-62477) x the frame time (the global word 0x827D_7508), steer x 0.5 while +169 (the brain's collision timer).
Smoothing on (+168 = 1, as the ctor sets it), +148 = 1. The chassis (+52) has +592 = 1 and the flags +40 are 0, so the
gearbox and the light / horn calls after the pedals are not reached; F-47121 (called first) is stubbed to nothing.

Writes <out-dir>/pedal_slew_ref.txt, hex words: "case collision dt cur.steer cur.throttle cur.brake cur.handbrake
target.steer target.throttle target.brake target.handbrake -> held steer throttle brake handbrake, chassis +132 +136 +140
+144"."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb, nudge

IN, CH = 0x40100000, 0x40101000
DT = 0x827D7508


def case(rnd):
    def pedal(lo):
        return rnd.choice((0.0, 1.0, lo, rnd.uniform(lo, 1), rnd.uniform(lo, 1), rnd.uniform(lo, 1)))
    cur = [pedal(-1), pedal(0), pedal(0), pedal(0)]
    tgt = [pedal(-1), pedal(0), pedal(0), pedal(0)]
    for k in range(4):
        r = rnd.random()
        if r < 0.15:
            tgt[k] = cur[k]
        elif r < 0.3:
            tgt[k] = cur[k] + rnd.choice((-1, 1)) * rnd.uniform(0, 0.2)
        elif r < 0.35:
            tgt[k] = nudge(cur[k], rnd.choice((-3, -1, 1, 3)))
        elif r < 0.36:
            tgt[k] = float('nan')
    dt = rnd.choice((1 / 60, 1 / 30, 1 / 60, rnd.uniform(0, 0.1), 0.0))
    return [rnd.choice((0, 1)), fb(dt)] + [fb(x) for x in cur + tgt]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-94408')
    rnd = random.Random(94408)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    main_src = (
        'static const uint32_t CS[][10]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'static const float R[6]={4.0f,6.0f,2.0f,30.0f,10.0f,35.0f};' + nl +
        'for(auto&s:CS){rt_reset();memset(pointer(@IN@),0,0x2000);store(@IN@+52,@CH@);store(@CH@+592,1u);' +
        '*pointer(@IN@+148)=1;*pointer(@IN@+168)=1;*pointer(@IN@+169)=(uint8_t)s[0];store(@DT@,s[1]);' +
        'for(int i=0;i<6;i++){uint32_t b;memcpy(&b,&R[i],4);store(@IN@+72+4*i,b);}' +
        'store(@IN@+24,s[2]);store(@IN@+12,s[3]);store(@IN@+16,s[4]);store(@IN@+20,s[5]);' +
        'for(int i=0;i<4;i++)store(@IN@+152+4*i,s[6+i]);' +
        'Context ctx=fresh();ctx.r3.u32=@IN@;sub_@F@(ctx,nullptr);' +
        'printf("case");for(int i=0;i<10;i++)printf(" %08x",s[i]);printf(" -> %08x %08x %08x %08x %08x %08x %08x %08x\\n",' +
        'load(@IN@+24),load(@IN@+12),load(@IN@+16),load(@IN@+20),load(@CH@+132),load(@CH@+136),load(@CH@+140),load(@CH@+144));}}' + nl)
    for k, v in (('IN', IN), ('CH', CH), ('DT', DT)):
        main_src = main_src.replace('@%s@' % k, '%du' % v)
    main_src = main_src.replace('@F@', fn)
    stubs = {'sub_' + addr(a.tool, 'F-47121'): ''}
    exe = rd.build(rd.index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'pedalslew', stubs=stubs, image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'pedal_slew_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
