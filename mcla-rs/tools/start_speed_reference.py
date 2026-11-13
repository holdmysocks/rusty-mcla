"""The start speed (car vfunc 84 F-30683) run offline: the generated code whole and unchanged on flat guest memory
(vector build: the slot test's bound size is a vsubfp128), with the junction-lane test F-50269 and the lane-to-junction
F-41621 in the closure. Stubbed, as inputs of the case: the network test F-24338 (false: D7), the car ahead / behind
F-90496 (the case's two cars, or none: run whole in start_neighbours_reference.py), the place getter F-58860 (the case's
places), the bound getter F-88657 (a bound whose slot 24 gives the case's length), the green test F-78185 and the
junction block F-50959 (the case's), the slot test F-12899 (the case's answer: run in slot_test_reference.py), the
removal F-51512 (records its reason). The behaviour's slot 40, the rails' slot 68 (speed) and the next rail's slot 48
(its first control point) are dispatched to the case's values.

Memory: our car 0x1000 (+48 the params pointer copy, +412 / +416 the speed, +424 offset, +448 type 0x1C00, +452
behaviour 0x1800, +500 byte), the call's r4 0x1400 (+128 the path params 0x1500: +152 speed mph, +164 bParked, +188
referenced, +189 fired), behaviour 0x1800 (vtable 0x1900; +188 rail 0x2000, +192 next 0x2400, +208 phase), the rails
(vtables 0x2100 / 0x2500, +8 owner 0x2200 / 0x2600 with +4 flags and +8 parent 0x2300 / 0x2700 with +4 flags), the type
0x1C00 (+104 0x1D00 -> +12 0x1D80, +108 bIsProxy, +111 / +112, +148 fMaxDecel), the cars ahead 0x3000 / behind 0x3400
(+208 velocity, +448 types 0x3200 / 0x3600), places 0x4000 / 0x4010 / 0x4020, the next rail's first point 0x4030.

A case (hex words): params, speed, b164, b188, b189, beh, rail, owner, oflags, parent, pflags, railspeed, offset,
phase, ahead, behind, pos xyz, len, maxdecel, ahead pos xyz, vel xyz, len, behind pos xyz, vel xyz, len, maxdecel, next,
nowner, noflags, nparent, npflags, nspeed, p0 xyz, green, blocked, has104, has12, clear, b500, b111, b112, b108.
Writes <out-dir>/start_speed_ref.txt: the case's words '->' +412 +416 +48 and the removal reason (0: none), hex."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x10000)
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic const uint32_t *K;\n')
NL = chr(10)
W = 54


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def case(r):
    odd = r.random() < 0.03
    speeds = (0.0, 2.68224, 4.4704, 6.7056, 8.9408, 13.4112, 15.6464, 20.1168, 26.8224)
    c = [int(r.random() < 0.8), fb(r.choice((0.0, 10.0, 25.0, 40.0, -5.0, r.uniform(0, 80)))),
         int(r.random() < 0.2), int(r.random() < 0.4), int(r.random() < 0.5)]
    beh = int(r.random() < 0.93)
    rail = int(beh and r.random() < 0.92)

    def owner():
        op = int(r.random() < 0.95)
        pp = int(op and r.random() < 0.5)
        return [op, r.choice((0, 0, 4, 0x10, 0x14, 0x1000)) if op else 0, pp, r.choice((0, 0, 4)) if pp else 0]
    sp = r.choice(speeds) if r.random() < 0.8 else r.uniform(0, 30)
    off = r.choice((0.0, -2.0, 1.5, r.uniform(-3, 5), -40.0))
    c += [beh, rail] + owner() + [fb(sp), fb(off), fb(r.random())]
    has_a, has_b = int(r.random() < 0.6), int(r.random() < 0.6)
    pos = [r.uniform(-50, 50), r.uniform(-2, 2), r.uniform(-50, 50)]
    ln = r.choice((4.5, 0.0, r.uniform(3, 6), r.uniform(3, 18)))
    md = r.choice((22.5, 8.0, r.uniform(2, 30), 0.0, -3.0))
    c += [has_a, has_b] + [fb(x) for x in pos] + [fb(ln), fb(md)]

    def other():
        lo = r.choice((4.5, r.uniform(3, 6), 0.0, r.uniform(3, 18)))
        g = r.choice((2.0, f32(2.0 * 1.0000001), f32(2.0 * 0.9999999), r.uniform(1.5, 2.5), r.uniform(-5, 40), r.uniform(0, 8)))
        dd = g + lo * 0.5 + ln * 0.5
        a = r.uniform(0, 2 * math.pi)
        p = [pos[0] + dd * math.cos(a), pos[1] + r.uniform(-0.3, 0.3), pos[2] + dd * math.sin(a)]
        s = r.choice((0.0, r.uniform(0, 5), r.uniform(0, 30)))
        b = r.uniform(0, 2 * math.pi)
        v = [s * math.cos(b), r.uniform(-0.5, 0.5) if s else 0.0, s * math.sin(b)]
        if odd and r.random() < 0.3:
            v[r.randrange(3)] = r.choice((float('nan'), float('inf'), 3.4028234663852886e+38))
        return [fb(x) for x in p] + [fb(x) for x in v] + [fb(lo)]
    c += other()
    c += other() + [fb(r.choice((22.5, 8.0, r.uniform(2, 30), 0.0, -3.0)))]
    nx = int(r.random() < 0.75)
    ns = r.choice(speeds) if r.random() < 0.8 else r.uniform(0, 30)
    dd = r.choice((ln * 0.5, f32(ln * 0.5 * 1.0000001), r.uniform(0, 3), r.uniform(0, 60)))
    a = r.uniform(0, 2 * math.pi)
    p0 = [pos[0] + dd * math.cos(a), pos[1] + r.uniform(-0.3, 0.3), pos[2] + dd * math.sin(a)]
    if odd and r.random() < 0.3:
        p0[r.randrange(3)] = float('nan')
    c += [nx] + owner() + [fb(ns)] + [fb(x) for x in p0] + [int(r.random() < 0.5), int(r.random() < 0.3)]
    c += [int(r.random() < 0.9), int(r.random() < 0.9), int(r.random() < 0.7),
          r.choice((0, 0, 0, 0x20, 0x1F, 0xFF)), int(r.random() < 0.1), int(r.random() < 0.1), int(r.random() < 0.08)]
    assert len(c) == W, len(c)
    return c


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    if not (1 <= a.cases <= 50000):
        raise SystemExit('--cases: 1 .. 50000')
    if not a.image.is_file() or a.image.stat().st_size < 0x900000:
        raise SystemExit('--image: the original image (work/2026-10-10/original-image.bin)')
    fn, net, nb, place, bound, green, block, slot, remove = (addr(a.tool, f) for f in (
        'F-30683', 'F-24338', 'F-90496', 'F-58860', 'F-88657', 'F-78185', 'F-50959', 'F-12899', 'F-51512'))
    r = random.Random(30683)
    cs = [case(r) for _ in range(a.cases)]
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x9100){}else if(a==0x9104)ctx.f1.f64=getf(0x1E00);else if(a==0x9108)ctx.f1.f64=getf(0x1E04);'
            'else if(a==0x910C)ctx.r3.u32=0x4030;else if(a==0x9110)ctx.f1.f64=getf(ctx.r3.u32+4);'
            'else throw std::runtime_error("unexpected indirect call");}')
    setup = ('memset(objects,0,sizeof(objects));K=c;'
             # params
             'store(0x1400+128,c[0]?0x1500:0);store(0x1500+152,c[1]);objects[0x1500-0x1000+164]=c[2];'
             'objects[0x1500-0x1000+188]=c[3];objects[0x1500-0x1000+189]=c[4];'
             # car, behaviour, rails
             'store(0x1000+424,c[12]);store(0x1000+448,0x1C00);store(0x1000+452,c[5]?0x1800:0);objects[0x1000-0x1000+500]=c[50];'
             'store(0x1800,0x1900);store(0x1900+40,0x9100);store(0x1800+188,c[6]?0x2000:0);store(0x1800+192,c[36]?0x2400:0);'
             'store(0x1800+208,c[13]);'
             'store(0x2000,0x2100);store(0x2100+68,0x9104);store(0x2000+8,c[7]?0x2200:0);store(0x2204,c[8]);'
             'store(0x2208,c[9]?0x2300:0);store(0x2304,c[10]);store(0x1E00,c[11]);'
             'store(0x2400,0x2500);store(0x2500+68,0x9108);store(0x2500+48,0x910C);store(0x2400+8,c[37]?0x2600:0);'
             'store(0x2604,c[38]);store(0x2608,c[39]?0x2700:0);store(0x2704,c[40]);store(0x1E04,c[41]);'
             'store(0x4030,c[42]);store(0x4034,c[43]);store(0x4038,c[44]);'
             # type, bound
             'store(0x1C00+104,c[47]?0x1D00:0);store(0x1D00+12,c[48]?0x1D80:0);objects[0x1C00-0x1000+108]=c[53];'
             'objects[0x1C00-0x1000+111]=c[51];objects[0x1C00-0x1000+112]=c[52];store(0x1C00+148,c[20]);'
             'store(0x4100+24,0x9110);store(0x1080,0x4100);store(0x1084,c[19]);store(0x3080,0x4100);store(0x3084,c[27]);'
             'store(0x3480,0x4100);store(0x3484,c[34]);'
             # places, the other cars
             'for(int k=0;k<3;k++){store(0x4000+4*k,c[16+k]);store(0x4010+4*k,c[21+k]);store(0x4020+4*k,c[28+k]);'
             'store(0x3000+208+4*k,c[24+k]);store(0x3400+208+4*k,c[31+k]);}'
             'store(0x3000+448,0x3200);store(0x3400+448,0x3600);store(0x3600+148,c[35]);')
    stubs = {
        'sub_' + net: 'ctx.r3.u32=0;',
        'sub_' + nb: 'store(ctx.r4.u32,K[14]?0x3000:0);store(ctx.r5.u32,K[15]?0x3400:0);',
        'sub_' + place: 'uint32_t x=ctx.r3.u32;ctx.r3.u32=x==0x1000?0x4000:x==0x3000?0x4010:x==0x3400?0x4020:0;'
                        'if(!ctx.r3.u32)throw std::runtime_error("place of an unknown car");',
        'sub_' + bound: 'ctx.r3.u32=ctx.r3.u32+0x80;',
        'sub_' + green: 'if(ctx.r4.u32!=0x2400)throw std::runtime_error("green: not the next rail");ctx.r3.u32=K[45];',
        'sub_' + block: 'if(ctx.r4.u32!=0x1000)throw std::runtime_error("block: not our car");ctx.r3.u32=K[46];',
        'sub_' + slot: 'if(ctx.r7.u32!=0x1000||ctx.r3.u32!=0x2000)throw std::runtime_error("slot: args");ctx.r3.u32=K[49];',
        'sub_' + remove: 'store(0x4200,ctx.r5.u32);',
    }
    rows = NL.join('{%s},' % ','.join('0x%08xu' % x for x in c) for c in cs)
    body = ('static const uint32_t C[][%d]={' % W + NL + rows + NL + '};' + NL + disp + NL +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + NL +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;'
            'sub_' + fn + '(ctx,nullptr);printf("%08x %08x %08x %xEOL",load(0x1000+412),load(0x1000+416),load(0x1000+48),load(0x4200));}}' + NL
            ).replace('EOL', chr(92) + 'n')
    exe = build(index(a.recomp), ['sub_' + fn], body, a.out_dir, 'start_speed', stubs=stubs, vector=True, scratch_end=0x10000)
    out = [x for x in run(exe, a.image).split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'start_speed_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases,', sum(o.split()[3] == '11' for o in out), 'removed')


if __name__ == '__main__':
    main()
