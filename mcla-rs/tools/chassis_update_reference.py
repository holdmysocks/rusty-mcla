"""The chassis update F-97237 (mcCarSim's per-tick update after the vehicle's) run offline on the generated code: the
function whole, its callees stubbed, over random chassis states, so that each of its own pieces can be compared:
  1. +576 bit 2 from the player options' byte (+797 of [[[0x8286F1D8] +700] +8]);
  2. the centre of mass +112: wiped out (+592 0x10) with UpsetCG (tune +268) > 0 out of the water: CenterOfMass +48 +
     (ModelOffset +32 - CenterOfMass) x (clamp(timer / WipeoutDelay, 0, 1) x UpsetCG); damaged out (0x04) likewise with
     UpsetCG alone; else its y (+116) goes towards CenterOfMass.y (tune +52) at dt x 0.5;
  3. the air time +584 (slot 11 = no wheel on the ground and mcCarStuck at rest: += dt; ending: the player's jump
     statistics, LandingDampFactor (tune +300) into the aero object's +4 above 0.25, negated; else 0 and +640 =
     the car's position);
  4. the still timer +588 (pedals 0, every wheel down, slow, not turning, no hydraulics or air ride button held);
  5. slot 13 below 0: the transmission's +28 = 0;
  6. the aero factor (aero +4): out of the water with a gyro on four wheels and in the air longer than AeroDampDelay
     (tune +312) or upside down (+260 < 0): AeroDampFactor (+304); else one dt towards 1;
  7. +578 > 0: [+16] +8 held at most +580 x [+16] +72;
  8. the calls after (the drivetrain F-79815's, nitro, SST, hydraulics, air ride, F-23458) - recorded, not run.
Stubbed: the water test F-12574 (the case's), the statistics' gate (0: not taken), the hydraulics' fitted test (0),
the chassis' slots 11 / 12 / 13 (the case's ground count and drive flag, and +584 - the getter decoded before).

Writes <out-dir>/chassis_update_ref.txt: "words(in) -> +576 +112..+124 +584 +588 +640..+652 aero+4 trans+28 [+16]+8
calls" (hex; calls a letter string)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)
import artic_reference  # noqa: F401 (its shim: the SDK helpers the vector code needs)

addr = bs.addr


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def f(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


HARNESS = r'''
#include <sstream>
enum { SIM=0x1000, VT=0x1400, CAR=0x1500, X20=0x1600, X20B=0x1680, MAT=0x1700, STK=0x1800, GYR=0x1900, PMR=0x1A00,
       AERO=0x1B00, FLU=0x1B80, S16=0x1C00, TR=0x1C80, BODY=0x2400, TUNE=0x1E00, HYD=0x2000, AIR=0x2100,
       G1=0x2200, G2=0x2280, G3=0x2300, STACK=0x7F00 };
static uint32_t g_ground, g_drives, g_water; static std::string g_calls;
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    if(a==0x0F00002C) ctx.r3.u64=g_ground;
    else if(a==0x0F000030) ctx.r3.u64=g_drives;
    else if(a==0x0F000034) { uint32_t v=load(SIM+584); float x; std::memcpy(&x,&v,4); ctx.f1.f64=x; }
    else { std::printf("indirect %08x\n",a); std::exit(5); }
}
int main(int argc,char **argv) {
    std::ifstream bin(argv[2],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    std::ifstream cases(argv[1]); std::string line;
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        std::memset(objects,0,sizeof(objects)); g_calls.clear();
        int i=0; auto nx=[&]{ return w[i++]; };
        uint32_t flags=nx(); g_water=nx(); uint32_t opt=nx(); uint32_t f576=nx();
        store(SIM+592,flags); *pointer(G3+797)=uint8_t(opt); store16(SIM+576,uint16_t(f576));
        store(0x8286F1D8,G1); store(G1+700,G2); store(G2+8,G3);
        store(SIM+0,VT); store(VT+44,0x0F00002C); store(VT+48,0x0F000030); store(VT+52,0x0F000034);
        store(SIM+4,CAR); store(CAR+20,X20); store(X20+8,X20B); store(CAR+28,MAT); store(CAR+128,PMR);
        store(SIM+8,AERO); store(SIM+12,FLU); store(SIM+16,S16); store(SIM+20,TR); store(SIM+128,BODY); store(SIM+368,TUNE);
        store(SIM+672,HYD); store(SIM+676,AIR);
        for(int j=0;j<4;j++) store(TUNE+32+4*j,nx());
        for(int j=0;j<4;j++) store(TUNE+48+4*j,nx());
        store(TUNE+268,nx()); store(X20+44,nx()); store(X20B+56,nx());
        for(int j=0;j<4;j++) store(SIM+112+4*j,nx());
        store(0x827D7508,nx());
        g_ground=nx(); store(SIM+24,nx());
        uint32_t stk=nx(); if(stk) { store(CAR+44,STK); store(STK+56,nx()); } else nx();
        store(SIM+584,nx());
        for(int j=0;j<4;j++) store(SIM+640+4*j,nx());
        for(int j=0;j<3;j++) store(MAT+64+4*j,nx());
        store(SIM+136,nx()); store(SIM+140,nx()); store(SIM+224,nx());
        for(int j=0;j<3;j++) store(BODY+288+4*j,nx());
        for(int j=0;j<4;j++) *pointer(HYD+48+j)=uint8_t(nx());
        for(int j=0;j<4;j++) *pointer(AIR+120+j)=uint8_t(nx());
        store(AIR+16,nx());
        g_drives=nx();
        uint32_t gy=nx(); if(gy) { store(CAR+48,GYR); store16(GYR+16,uint16_t(nx())); store16(GYR+18,uint16_t(nx())); } else { nx(); nx(); }
        store(TUNE+300,nx()); store(TUNE+304,nx()); store(TUNE+312,nx());
        store(SIM+260,nx()); store(AERO+4,nx());
        *pointer(SIM+578)=uint8_t(nx()); store(SIM+580,nx()); store(S16+8,nx()); store(S16+72,nx());
        store(TR+28,nx());
        Context ctx{}; ctx.r1.u32=STACK; ctx.r3.u32=SIM;
        FN(ctx,nullptr);
        std::printf("%04x",load16(SIM+576));
        for(int j=0;j<4;j++) std::printf(" %08x",load(SIM+112+4*j));
        std::printf(" %08x %08x",load(SIM+584),load(SIM+588));
        for(int j=0;j<4;j++) std::printf(" %08x",load(SIM+640+4*j));
        std::printf(" %08x %08x %08x %s\n",load(AERO+4),load(TR+28),load(S16+8),g_calls.empty()?"-":g_calls.c_str());
    }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    fn = sub('F-97237')
    rnd = random.Random(97237)
    U = lambda lo, hi: bits(f(rnd.uniform(lo, hi)))
    cases = []
    for _ in range(a.cases):
        c = [rnd.choice([0, 0, 0x10, 0x04, 0x14, 0x01, 0x02]), rnd.choice([0, 0, 1]), rnd.choice([0, 1]), rnd.choice([0, 2, 5, 0xFFFF])]
        c += [U(-1, 1), U(-0.5, 1), U(-2, 2), U(-1, 1)]
        c += [U(-1, 1), U(-0.5, 1), U(-2, 2), U(-1, 1)]
        c += [rnd.choice([0, bits(0.0), U(0, 1), U(-0.5, 0), bits(1.0)]), U(-1, 4), rnd.choice([U(0.5, 3), bits(0.0)])]
        c += [U(-1, 1), U(-0.5, 1.5), U(-2, 2), U(-1, 1)]
        c += [rnd.choice([bits(1 / 60), bits(1 / 30), U(0, 0.05)])]
        c += [rnd.choice([0, 0, 1, 2, 4]), rnd.choice([4, 4, 2])]
        c += [rnd.choice([0, 1, 1]), rnd.choice([0, 0, 2])]
        c += [rnd.choice([bits(0.0), U(0, 2), U(-1, 0), U(0, 0.3)])]
        c += [U(-50, 50) for _ in range(4)] + [U(-50, 50) for _ in range(3)]
        c += [rnd.choice([bits(0.0), bits(0.0), U(0, 1)]), rnd.choice([bits(0.0), bits(0.0), U(0, 1)]), rnd.choice([U(0, 3), U(0, 30)])]
        c += [rnd.choice([bits(0.0), U(-1, 1)]) for _ in range(3)]
        c += [rnd.choice([0, 0, 0, 1]) for _ in range(4)] + [rnd.choice([0, 0, 0, 1]) for _ in range(4)] + [rnd.choice([0, 0, 1])]
        c += [rnd.choice([0, 1])]
        c += [rnd.choice([1, 1, 0]), rnd.choice([0, 0, 1]), rnd.choice([0, 0, 1])]
        c += [U(0, 1), U(0, 2), rnd.choice([U(0, 1), bits(0.0), U(-0.5, 0)])]
        c += [rnd.choice([U(-1, 1), U(0.5, 1)]), rnd.choice([U(0, 2), bits(1.0)])]
        c += [rnd.choice([0, 0, 1, 2, 0xFF]), U(0, 2), U(0, 10), U(0, 10)]
        c += [U(0, 1)]
        cases.append(c)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'chassis_update_cases.txt'
    nl = chr(10)
    inp.write_text(nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             'static void dispatch(Context&,const uint8_t*,uint32_t);\n'
                             '#include <string>\nstatic uint32_t g_ground, g_drives, g_water; static std::string g_calls;\n')
    rec = lambda ch: 'g_calls+="%s";' % ch
    stubs = {
        sub('F-12574'): 'ctx.r3.u64=g_water;',
        sub('F-14919'): 'ctx.r3.u64=0;' + rec('s'),
        sub('F-65988'): 'ctx.r3.u64=0;' + rec('S'),
        sub('F-79815'): rec('d'),
        sub('F-84636'): rec('n'),
        sub('F-60935'): rec('t'),
        sub('F-57868'): 'ctx.r3.u64=0;' + rec('h'),
        sub('F-32886'): rec('j'), sub('F-94916'): rec('k'), sub('F-89410'): rec('l'), sub('F-93185'): rec('m'),
        sub('F-31409'): 'ctx.r3.u64=0;' + rec('q'),
        sub('F-23458'): rec('z'),
    }
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], HARNESS.replace('FN(', fn + '(').replace('static uint32_t g_ground, g_drives, g_water; static std::string g_calls;\n', ''),
                               a.out_dir, 'curef', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3'))
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'chassis_update_ref.txt').write_text(nl.join(lines) + nl)
    calls = {}
    for o in out:
        k = o.split()[-1]
        calls[k] = calls.get(k, 0) + 1
    print(len(lines), 'cases', calls)


if __name__ == '__main__':
    main()
