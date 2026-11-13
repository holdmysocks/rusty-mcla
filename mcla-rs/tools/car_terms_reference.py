"""The car's contact callback F-55998, its materials part, run offline on the generated code: one contact a case with an
instance that is not a vehicle or a prop (its type 11 - the thrown rider - or 3 / 1, fixed or not by its level state), so
the callback takes the branch after its vehicle part (A-31611 ..): the driver's record (+24 == 3: the player; index -1:
none), the two-wheel state (mcCarGyro +18), the car's matrix (car +272 or [car +28] +16 by the thread's byte +72), the
wiped-out bits (+592 0x10 / 0x04), the fixed partner's head-on test (the normal along the matrix's row 2 above cos(25
degrees), the speed +224 above 22.35) and the stuck state (car +44 -> +56 == 2 and the chassis' +20 -> +20). Evidence:
docs/reference/vehicle/car_terms_check.md.

Stubbed: the other instance's getter F-78981 (the case's), the dynamic cast F-75805 (the case's gyro), the instance's type
(a virtual call). The level table and the driver table are globals set to the case's.

Writes <out-dir>/car_terms_ref.txt: "words(in) -> friction elasticity own_y" (hex words)."""
import argparse
import math
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
enum { CAR=0x1100, CH=0x1200, SIM=0x1600, TUNE=0x1800, DRVX=0x1A00, TAB=0x1B00, GYRO=0x1D00, MAT=0x1E00, Y=0x1F00, S20=0x1F80,
       ITER=0x2000, ARR=0x2040, REC=0x2100, PAIR=0x2200, OTH=0x2300, VTO=0x2380, LEV=0x2400, LARR=0x2500, TLS=0x2600, TLSB=0x2700,
       STACK=0x7F00 };
static uint32_t g_type;
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    if(a==0x0F000004) ctx.r3.u64=g_type;
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
        std::memset(objects,0,sizeof(objects));
        // w: type state drvidx drvtype two thread upy r2x r2y r2z wiped speed stuck56 s20 car_a nx ny nz ya yb fr el off
        g_type=w[0];
        store(CAR+80,CH); store(CH+8,SIM); store(SIM+368,TUNE); store(TUNE+116,w[19]); store(TUNE+120,w[20]); store(TUNE+280,w[21]);
        store(CH+76,DRVX); store(DRVX+76,w[2]); store(0x828CD0A8,TAB); store(TAB+104*0+24,w[3]); store(TAB+104*1+24,w[3]);
        store(CH+48,0x1C00);
        store16(GYRO+18,uint16_t(w[4]));
        uint32_t m = w[5] ? CH+272 : MAT+16; store(CH+28,MAT);
        store(m+20,w[6]); store(m+32,w[7]); store(m+36,w[8]); store(m+40,w[9]);
        store(SIM+592,w[10]); store(SIM+224,w[11]);
        store(CH+44,Y); store(Y+56,w[12]); store(SIM+20,S20); store(S20+20,w[13]);
        store(ITER,ARR); store(ITER+4,0x3000); store(ITER+8,0); store(ITER+12,1); store(ARR,REC);
        *pointer(REC+165)=0xC0; store(REC+160,PAIR); store(PAIR,w[14]?0x3000:0x3100);
        store(REC+80,w[15]); store(REC+84,w[16]); store(REC+88,w[17]);
        store(REC+96,w[15]^0x80000000u); store(REC+100,w[16]^0x80000000u); store(REC+104,w[17]^0x80000000u);
        store(REC+52,w[18]); store(REC+68,w[22]); store(REC+28,0x11111111); store(REC+44,0x22222222);
        store(OTH,VTO); store(VTO+4,0x0F000004); store16(OTH+8,5); store(0x828DA464,LEV); store(LEV+112,LARR); store(LARR+5*8+4,w[1]);
        store(TLS,TLSB); *pointer(TLSB+72)=uint8_t(w[5]);
        Context ctx{}; ctx.r1.u32=STACK; ctx.r13.u32=TLS; ctx.r3.u32=CAR; ctx.r4.u32=ITER;
        FN(ctx,nullptr);
        std::printf("%08x %08x %08x %08x\n",load(REC+28),load(REC+44),load(REC+52),load(REC+68));
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
    fn = sub('F-55998')
    rnd = random.Random(55998)
    cases = []
    c45 = struct.unpack('<f', struct.pack('<I', 0x3f3504e6))[0]
    c25 = f(math.cos(0.4363323152065277))
    for _ in range(a.cases):
        ty = rnd.choice([11, 11, 3, 3, 1])
        state = rnd.choice([2, 2, 1, 0, 3])
        drv = rnd.choice([0, 1, 0xFFFFFFFF])
        dtype = rnd.choice([3, 3, 1, 2])
        two = rnd.choice([0, 0, 0, 1, 0xFFFF])
        upy = f(rnd.choice([rnd.uniform(-1, 1), c45, f(c45) * 0.9999999, 0.0, -0.0, 1.0, -1.0]))
        n = [rnd.uniform(-1, 1) for _ in range(3)]
        l = math.sqrt(sum(x * x for x in n)) or 1
        n = [f(x / l) for x in n]
        r2 = [rnd.uniform(-1, 1) for _ in range(3)]
        if rnd.random() < 0.5:
            r2 = [n[i] * rnd.choice([1, -1]) + rnd.uniform(-0.3, 0.3) for i in range(3)]
        l = math.sqrt(sum(x * x for x in r2)) or 1
        r2 = [f(x / l) for x in r2]
        if rnd.random() < 0.05:
            # (along on cos 25 degrees: the normal r2 itself turned so that n . r2 = c25 to the float)
            pass
        wiped = rnd.choice([0, 0, 0, 0x10, 0x04, 0x14, 0x01, 0x08])
        speed = f(rnd.choice([rnd.uniform(0, 50), 22.35, f(22.35) * 1.0000001, f(22.35) * 0.9999999]))
        stuck56 = rnd.choice([2, 2, 0, 1])
        s20 = rnd.choice([0, 1, 5])
        car_a = rnd.choice([0, 1])
        ya, yb = f(rnd.uniform(-50, 50)), f(rnd.uniform(-50, 50))
        fr, el, off = f(rnd.uniform(0, 2)), f(rnd.uniform(0, 1)), f(rnd.uniform(0, 1))
        cases.append([ty, state, drv, dtype, two, rnd.choice([0, 1]), bits(upy)] + [bits(x) for x in r2] +
                     [wiped, bits(speed), stuck56, s20, car_a] + [bits(x) for x in n] + [bits(ya), bits(fr), bits(el), bits(off), bits(yb)])
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'car_terms_cases.txt'
    nl = chr(10)
    inp.write_text(nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             'static void dispatch(Context&,const uint8_t*,uint32_t);\n')
    stubs = {sub('F-78981'): 'ctx.r3.u64=0x2300;', sub('F-75805'): 'ctx.r3.u64=0x1D00;'}
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], HARNESS.replace('FN(', fn + '('), a.out_dir, 'tref', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3'))
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'car_terms_ref.txt').write_text(nl.join(lines) + nl)
    kinds = {}
    for o in out:
        k = ('set' if o.split()[0] != '11111111' else 'kept') + (' f0' if o.split()[0] == '00000000' else '')
        kinds[k] = kinds.get(k, 0) + 1
    print(len(lines), 'cases', kinds)


if __name__ == '__main__':
    main()
