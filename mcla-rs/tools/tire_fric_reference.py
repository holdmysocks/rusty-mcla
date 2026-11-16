"""mcCarTireFric::Update F-67816 run offline on the generated code: the tyres' melt (rise at MeltRate while eligible and the
throttle share x the worst driven wheel's slide is over SkiddingThreshold, else down at RecoverRate, lifted back to 0 at the
end), the all-wheel-drive layout switch through the setter F-82657 (a stub here: logs its argument and writes chassis +372, as
the real one does before it rebuilds the drivetrain), and the wheels' +468 / +472. Eligible: gear (transmission +20 less 1) in
1 .. GearThreshold, no burnout (chassis +580), not (throttle < 0.5 and brake > 0.1), not (throttle < 0.1 and handbrake +144 >
0.1), |chassis +572| >= 5, and not the car's ability running Agro (mcCar +72: +52 == 1 and +48 == 3). The port's decode:
physics::vehsim::tire_fric_update (test `tire_fric_matches_the_original`).

Memory: this 0x1000 (+4 tune 0x1100, +8 chassis 0x1200), chassis +4 mcCar 0x1400, +20 transmission 0x1500, +28 wheel list
0x1600 (4 wheels at 0x2000 + 0x400 k, their tunes at +0x200), +368 car tune 0x1700, +400 drivetrain 0x1800 (+20 count, +24 the
driven wheels: by the layout, rear for 0, front for 1 and 4, all for 2 and 3), mcCar +72 the ability 0x1900. dt at the frame-time global (G +8).

A case line: "<inputs> -> melt, layout, setter calls (count then up to 2 arguments), the 4 wheels' +468 and +472" (hex).
Inputs: melt, gear index, threshold, skid, rate, recover, spinning, throttle, brake, hand, shaft, burn, drivetrain type,
layout, ability (0 none, else type << 8 | state), the 4 slides, flag +20, dt, static, sliding (front), static, sliding (back)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the recomp's x86 flags)

addr = bs.addr


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


HARNESS = r'''
#include <sstream>
#include <string>
static void put8(uint32_t a,uint8_t v) { *pointer(a)=v; }
int main(int argc,char **argv) {
    std::ifstream bin(argv[2],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    std::ifstream cases(argv[1]); std::string line;
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> c; std::string tok;
        while(in>>tok) c.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(c.size()<25) continue;
        std::memset(objects,0,sizeof(objects)); NCALL=0;
        store(0x1000+4,0x1100); store(0x1000+8,0x1200); store(0x1000+12,0x3f8ccccd); store(0x1000+16,0x3f666666);
        store(0x1000+20,c[19]); store(0x1000+24,c[0]); put8(0x1000+28,1);
        store(0x1100+24,c[2]); store(0x1100+28,c[3]); store(0x1100+32,c[4]); store(0x1100+36,c[5]); store(0x1100+40,c[6]);
        store(0x1200+4,0x1400); store(0x1200+20,0x1500); store(0x1500+20,c[1]);
        store(0x1200+24,4); store(0x1200+28,0x1600);
        for(int k=0;k<4;k++){ uint32_t w=0x2000+0x400*k; store(0x1600+4*k,w); store(w+4,w+0x200); store(w+520,c[15+k]);
            store(w+0x200+116,c[k<2?21:23]); store(w+0x200+120,c[k<2?22:24]); }
        store(0x1200+136,c[7]); store(0x1200+140,c[8]); store(0x1200+144,c[9]); store(0x1200+572,c[10]); store(0x1200+580,c[11]);
        store(0x1200+368,0x1700); store(0x1700+132,c[12]); store(0x1200+372,c[13]); store(0x1200+400,0x1800);
        { int lay=int(c[13]); int n=0; for(int k=0;k<4;k++){ bool d = lay==0 ? k>=2 : (lay==1||lay==4) ? k<2 : true;
            if(d){ store(0x1800+24+4*n,0x2000+0x400*k); n++; } } store(0x1800+20,n); }
        if(c[14]) { store(0x1400+72,0x1900); store(0x1900+52,c[14]>>8); store(0x1900+48,c[14]&0xFF); }
        store((0x827Du<<16)|0x7508u,c[20]);
        Context ctx{}; ctx.r1.u32=0x7F00; ctx.r3.u32=0x1000;
        FN(ctx,nullptr);
        std::printf("%x %x %x %x %x",load(0x1000+24),load(0x1200+372),NCALL,CALLS[0],CALLS[1]);
        for(int k=0;k<4;k++){ uint32_t w=0x2000+0x400*k; std::printf(" %x %x",load(w+468),load(w+472)); }
        std::printf("\n");
    }
}
'''


def case(rnd):
    ch, U = rnd.choice, rnd.uniform
    f = lambda x: bits(x)
    tf = lambda lo, hi: f(U(lo, hi))
    melt = ch([0.0, 0.0, U(0, 1), U(0, 0.01), 1.0, U(0.9, 1.0), 1e-4])
    gear = ch([-1, 0, 1, 2, 2, 3, 3, 4, 5, 6, 7])
    thr = ch([1, 2, 2, 3, 3, 4, 0])
    skid = ch([U(0, 1), U(0.3, 0.9), 0.5, 1.0, 1.2])
    rate = ch([U(0.1, 3), 0.0, U(0.1, 1)])
    rec = ch([U(0.1, 3), U(0.05, 0.5)])
    spin = ch([U(0.3, 2), 0.0, U(0.5, 1.5)])
    thro = ch([1.0, 1.0, U(0, 1), 0.5, 0.0, 0.1, 0.09, 0.75])
    brake = ch([0.0, 0.0, U(0, 1), 0.1, 0.11, 1.0])
    hand = ch([0.0, 0.0, 0.0, 1.0, U(0, 1), 0.1, 0.11, 0.05])
    shaft = ch([U(-80, 80), U(-10, 10), 5.0, -5.0, 4.99, 0.0, U(20, 120)])
    burn = ch([0.0, 0.0, 0.0, U(0, 2), -0.5])
    dtype = ch([0, 1, 2, 2, 3, 3])
    lay = ch([dtype, dtype, 0, 1, 2, 3, 4])
    ab = ch([0, 0, 0x103, 0x103, 0x102, 0x203, 0x100 | rnd.randrange(5)])
    slides = [ch([0.0, U(0, 1), U(0, 2), U(0.5, 1.5), 1.0]) for _ in range(4)]
    flag = ch([0, 0, 1])
    dt = ch([1 / 60, 1 / 30, U(0.001, 0.05)])
    st_f, sl_f = U(0.8, 2.5), U(0.5, 2.0)
    st_b, sl_b = U(0.8, 2.5), U(0.5, 2.0)
    return ([f(melt), gear + 1, thr, f(skid), f(rate), f(rec), f(spin), f(thro), f(brake), f(hand), f(shaft), f(burn), dtype, lay, ab]
            + [f(x) for x in slides] + [flag, f(dt), f(st_f), f(sl_f), f(st_b), f(sl_b)])


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    fn = sub('F-67816')
    rnd = random.Random(67816)
    cases = [case(rnd) for _ in range(a.cases)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'tire_fric_cases.txt'
    nl = chr(10)
    inp.write_text(nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += '\nstatic uint32_t NCALL, CALLS[4];\n'
    stubs = {sub('F-82657'): '{ if(NCALL<4) CALLS[NCALL]=ctx.r4.u32; NCALL++; store(ctx.r3.u32+372,ctx.r4.u32); }'}
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], HARNESS.replace('FN(', fn + '('), a.out_dir, 'tirefric', stubs=stubs, vector=True,
                               scratch_end=0x8000, flags=('-msse4.1', '-mssse3'))
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'tire_fric_ref.txt').write_text(nl.join(lines) + nl)
    melted = sum(1 for o in out if int(o.split()[0], 16) not in (0, 0x80000000))
    print(len(lines), 'cases;', melted, 'with a melt after;', sum(1 for o in out if o.split()[2] != '0'), 'with a layout switch')


if __name__ == '__main__':
    main()
