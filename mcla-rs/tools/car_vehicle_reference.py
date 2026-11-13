"""The car's contact callback F-55998, its vehicle and prop parts, run offline on the generated code: one contact a
case whose partner is an mcCar (type 8), an ambient vehicle (6) or a prop (4 / 13). The callback lays the normal flat by
how upright the two stand (types 8 / 6 always, 4 / 13 when the normal as it pushes the car points down), then for a
vehicle sorts the contact by where the car meets it - in front and head-on, behind and the same way, from the side and
into it - writing normals, points and materials, and ends with its materials part (car_terms_reference.py): the tune's
for an mcCar, the driver branch for the rest. Evidence: docs/reference/vehicle/car_terms_check.md.

Stubbed: the other instance's getter F-78981 (the case's partner), the dynamic cast F-75805 (the case's gyro), the
instances' type (a virtual call) and slot 10 of the car's [+80] +128 object (answers 0: the player's statistics and the
landing-damping write it guards are not taken - see the evidence file).

Writes <out-dir>/car_vehicle_ref.txt: "words(in) -> rec +28 +44, +48..+60, +64..+76, +80..+88, +96..+104" (hex)."""
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
enum { CAR=0x1100, CH=0x1200, SIM=0x1600, TUNE=0x1800, DRVX=0x1A00, TAB=0x1B00, GYRO=0x1D00, Y=0x1F00, S20=0x1F80,
       ITER=0x2000, ARR=0x2040, REC=0x2100, PAIR=0x2200, OTH=0x2300, VTO=0x2380, LEV=0x2400, LARR=0x2500, TLS=0x2600, TLSB=0x2700,
       OWN=0x3000, COL=0x3200, O128=0x3400, V128=0x3480, STACK=0x7F00 };
static uint32_t g_type;
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    if(a==0x0F000004) ctx.r3.u64=g_type;
    else if(a==0x0F000028) ctx.r3.u64=0;
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
        // w: 0 type 1 state 2 drvidx 3 drvtype 4 two 5 wiped 6 speed 7 stuck56 8 s20 9 car_a 10 fr 11 el 12 off
        //    13..24 own rows+pos 25..36 other rows+pos 37..39 vel 40..42 normal 43..46 point A 47..50 point B
        g_type=w[0];
        store(CAR+80,CH); store(CH+8,SIM); store(SIM+368,TUNE); store(TUNE+116,w[10]); store(TUNE+120,w[11]); store(TUNE+280,w[12]);
        store(CH+76,DRVX); store(DRVX+76,w[2]); store(0x828CD0A8,TAB); store(TAB+104*0+24,w[3]); store(TAB+104*1+24,w[3]);
        store(CH+48,0x1C00); store(CH+128,O128); store(O128,V128); store(V128+40,0x0F000028);
        store16(GYRO+18,uint16_t(w[4]));
        store(CH+28,OWN);
        for(int i=0;i<4;i++) for(int j=0;j<3;j++) { store(OWN+16+16*i+4*j,w[13+3*i+j]); store(OTH+16+16*i+4*j,w[25+3*i+j]); }
        store(SIM+592,w[5]); store(SIM+224,w[6]);
        store(CH+44,Y); store(Y+56,w[7]); store(SIM+20,S20); store(S20+20,w[8]);
        store(ITER,ARR); store(ITER+4,OWN); store(ITER+8,0); store(ITER+12,1); store(ARR,REC);
        *pointer(REC+165)=0xC0; store(REC+160,PAIR); store(PAIR,w[9]?OWN:0x3100); store(PAIR+8,COL); store(PAIR+12,COL);
        for(int j=0;j<3;j++) store(COL+272+4*j,w[37+j]);
        store(REC+80,w[40]); store(REC+84,w[41]); store(REC+88,w[42]);
        store(REC+96,w[40]^0x80000000u); store(REC+100,w[41]^0x80000000u); store(REC+104,w[42]^0x80000000u);
        for(int j=0;j<4;j++) { store(REC+48+4*j,w[43+j]); store(REC+64+4*j,w[47+j]); }
        store(REC+28,0x11111111); store(REC+44,0x22222222);
        store(OTH,VTO); store(VTO+4,0x0F000004); store16(OTH+8,5); store(0x828DA464,LEV); store(LEV+112,LARR); store(LARR+5*8+4,w[1]);
        store(TLS,TLSB); *pointer(TLSB+72)=0;
        Context ctx{}; ctx.r1.u32=STACK; ctx.r13.u32=TLS; ctx.r3.u32=CAR; ctx.r4.u32=ITER;
        FN(ctx,nullptr);
        std::printf("%08x %08x",load(REC+28),load(REC+44));
        for(int j=0;j<8;j++) std::printf(" %08x",load(REC+48+4*j));
        for(int j=0;j<3;j++) std::printf(" %08x",load(REC+80+4*j));
        for(int j=0;j<3;j++) std::printf(" %08x",load(REC+96+4*j));
        std::printf("\n");
    }
}
'''


def rot(rnd):
    """A rotation's rows (right, up, back): mostly upright with a tilt, sometimes anything."""
    yaw = rnd.uniform(-math.pi, math.pi)
    tilt = rnd.choice([0.0, rnd.uniform(0, 0.4), rnd.uniform(0, math.pi)])
    ax = rnd.uniform(-math.pi, math.pi)
    cy, sy = math.cos(yaw), math.sin(yaw)
    ry = [[cy, 0, -sy], [0, 1, 0], [sy, 0, cy]]
    k = [math.cos(ax), 0, math.sin(ax)]
    c, s = math.cos(tilt), math.sin(tilt)

    def rodr(v):
        kv = sum(k[i] * v[i] for i in range(3))
        cr = [k[1] * v[2] - k[2] * v[1], k[2] * v[0] - k[0] * v[2], k[0] * v[1] - k[1] * v[0]]
        return [v[i] * c + cr[i] * s + k[i] * kv * (1 - c) for i in range(3)]
    return [[f(x) for x in rodr(r)] for r in ry]


def unit(v):
    l = math.sqrt(sum(x * x for x in v)) or 1
    return [f(x / l) for x in v]


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
    rnd = random.Random(55999)
    cases = []
    for _ in range(a.cases):
        ty = rnd.choice([8, 6, 6, 4, 13])
        state = rnd.choice([0, 1]) if ty in (8, 6) else rnd.choice([0, 1, 2])
        drv = rnd.choice([0, 1, 0xFFFFFFFF])
        dtype = rnd.choice([3, 3, 1, 2])
        two = rnd.choice([0, 0, 0, 1])
        wiped = rnd.choice([0, 0, 0, 0, 0x10, 0x04, 0x01])
        speed = f(rnd.choice([rnd.uniform(0, 50), 30.0, 10.0]))
        stuck56 = rnd.choice([2, 0, 1])
        s20 = rnd.choice([0, 1])
        car_a = rnd.choice([0, 1])
        fr, el, off = f(rnd.uniform(0, 2)), f(rnd.uniform(0, 1)), f(rnd.uniform(0, 1))
        own = rot(rnd)
        oth = rot(rnd)
        opos = [f(rnd.uniform(-100, 100)) for _ in range(3)]
        tpos = [f(opos[i] + rnd.uniform(-6, 6)) for i in range(3)]
        back = own[2]
        sp = rnd.uniform(0, 40)
        vel = [f(-back[i] * sp * rnd.choice([1, 1, -1]) + rnd.uniform(-8, 8)) for i in range(3)]
        if rnd.random() < 0.1:
            vel = [0.0, 0.0, 0.0]
        pick = rnd.random()
        if pick < 0.35:
            n = unit([oth[2][i] * rnd.choice([1, -1]) + rnd.uniform(-0.4, 0.4) for i in range(3)])
        elif pick < 0.6:
            n = unit([back[i] * rnd.choice([1, -1]) + rnd.uniform(-0.5, 0.5) for i in range(3)])
        else:
            n = unit([rnd.uniform(-1, 1) for _ in range(3)])
        pa = [f(tpos[i] + rnd.uniform(-3, 3)) for i in range(3)] + [f(rnd.uniform(-1, 1))]
        pb = [f(tpos[i] + rnd.uniform(-3, 3)) for i in range(3)] + [f(rnd.uniform(-1, 1))]
        row = [ty, state, drv, dtype, two, wiped, bits(speed), stuck56, s20, car_a, bits(fr), bits(el), bits(off)]
        row += [bits(x) for r in own for x in r] + [bits(x) for x in opos]
        row += [bits(x) for r in oth for x in r] + [bits(x) for x in tpos]
        row += [bits(x) for x in vel] + [bits(x) for x in n] + [bits(x) for x in pa] + [bits(x) for x in pb]
        cases.append(row)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'car_vehicle_cases.txt'
    nl = chr(10)
    inp.write_text(nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             'static void dispatch(Context&,const uint8_t*,uint32_t);\n')
    stubs = {sub('F-78981'): 'ctx.r3.u64=0x2300;', sub('F-75805'): 'ctx.r3.u64=0x1D00;'}
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], HARNESS.replace('FN(', fn + '('), a.out_dir, 'vref', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3'))
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'car_vehicle_ref.txt').write_text(nl.join(lines) + nl)
    kinds = {}
    for c, o in zip(cases, out):
        ws = o.split()
        k = '%d %s' % (c[0], 'set' if ws[0] != '11111111' else 'kept')
        kinds[k] = kinds.get(k, 0) + 1
    print(len(lines), 'cases', dict(sorted(kinds.items())))


if __name__ == '__main__':
    main()
