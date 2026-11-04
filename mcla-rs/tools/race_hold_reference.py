"""A riding traffic car's wanted speeds with the race holds (F-70324) run offline: the generated code whole and unchanged,
with the lane predicates F-50269, F-15116, F-73512 and the route constraint F-37240 in the closure, on flat guest memory
(scratch up to 0x80000, so the ambient manager's fields at +0x37350 .. +0x64251 fit). Stubbed, as inputs of the case:
the signal lane pick F-55442 (no signal lane: the port's signal stop is checked elsewhere), the network test F-24338
(false: D7), the remote-entity test F-23063 (the case's), the car's matrix block F-33765 (the case's heading at +32 / +40),
the lane-to-junction F-85925 (the case's junctions: checked on every city lane by itself, stage6), the velocity getter
F-88657 (the case's speed as (s, 0, 0)), the distance left F-83943 (the case's). The behaviour's slots 19 / 20 and the
lanes' slot 17 (the speed) are dispatched to the case's values. The context byte +16 is 0 (the port has no standing
path car on a lane) and the stoponclosedroads / spawnaroundcamera parameters are the image's (0).
Memory: the call's r3 0x1000 (+16 the car 0x1100: +424 offset, +452 behaviour 0x1400, +500 byte), the context 0x1080
(+0 behaviour, +4 manager S 0x10000, +8 car, +16 byte), behaviour vtable 0x1300, lanes 0x1600 / 0x1A00 (vtable 0x1700,
+4 flags, +8 owner, +0x40 the case's speed) with owners 0x1800 / 0x1B00 and parents 0x1900 / 0x1C00, entry record 0x1E40.
Writes <out-dir>/race_hold_ref.txt: the case's words, then '->', the returned speed and the record's +4 +8 +12 words
and +16 byte (hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic const uint32_t *K;\n')


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    want, lanepick, net, remote, block, into, vel, left = (addr(a.tool, f) for f in (
        'F-70324', 'F-55442', 'F-24338', 'F-23063', 'F-33765', 'F-85925', 'F-88657', 'F-83943'))
    rnd = random.Random(70324)
    flags_o = (0, 4, 0x10, 0x14, 0x20, 0x24, 0x30, 0x34, 0x40, 0x44, 0x1010, 0x1014)
    flags_l = (0, 0x1000, 0x2000, 0x3000, 0x800, 0x2800)
    speeds = (0.0, 2.68224, 4.4704, 6.7056, 8.9408, 13.4112, 15.6464, 20.1168, 26.8224)
    cases = []
    for _ in range(6000):
        ev = int(rnd.random() < 0.75)
        rem = int(rnd.random() < 0.06)
        b500 = 0x20 if rnd.random() < 0.06 else 0
        s19 = int(rnd.random() < 0.06)
        s20 = int(rnd.random() < 0.06)

        def lane():
            op = int(rnd.random() < 0.9)
            pp = int(op and rnd.random() < 0.5)
            return [op, rnd.choice(flags_o) if op else 0, pp, rnd.choice((0, 4, 0x10)) if pp else 0, rnd.choice(flags_l),
                    fb(rnd.choice(speeds) if rnd.random() < 0.8 else rnd.uniform(0, 30))]
        cur = lane()
        nx = int(rnd.random() < 0.85)
        nxt = lane() if nx else [0] * 6
        off = rnd.choice((0.0, -2.0, 1.5, rnd.uniform(-3, 5)))
        phase = rnd.choice((0.3, 0.29999998, 0.30000001, rnd.random()))
        ang = rnd.uniform(0, 6.283185)
        h = (struct.unpack('>f', struct.pack('>f', __import__('math').cos(ang)))[0], f32(__import__('math').sin(ang)))
        if rnd.random() < 0.3:
            # the dot about 0.7071: d at 45 degrees from h, one way or the other, a float step either side
            import math
            t = ang + rnd.choice((1, -1)) * math.pi / 4 + rnd.choice((0, math.pi))
            d = (f32(math.cos(t)), f32(math.sin(t)))
            if rnd.random() < 0.5:
                d = (f32(d[0] * rnd.choice((1.0000001, 0.9999999))), d[1])
        else:
            import math
            t = rnd.uniform(0, 6.283185)
            d = (f32(math.cos(t)), f32(math.sin(t)))
        jc = rnd.choice((0, 1, 1, 2))
        jn = rnd.choice((0, 1, 1, 2))
        p1 = int(rnd.random() < 0.85)
        sp = rnd.choice((0.0, 3.0, 4.4704, 8.0, rnd.uniform(0, 30)))
        dist = rnd.uniform(0, 120)
        cases.append([ev, rem, b500, s19, s20] + cur + [nx] + nxt + [fb(off), fb(phase), fb(h[0]), fb(h[1]), fb(d[0]), fb(d[1]),
                                                               jc, jn, p1, fb(sp), fb(dist)])
    nl = chr(10)
    NL = chr(92) + 'n'
    W = len(cases[0])
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x9100)ctx.r3.u32=K[3];else if(a==0x9104)ctx.r3.u32=K[4];'
            'else if(a==0x9108)ctx.f1.f64=getf(ctx.r3.u32+0x40);else if(a==0x910C)ctx.r3.u32=0x1FC0;'
            'else throw std::runtime_error("unexpected indirect call");}')
    lanes = ''
    for base, lw, ow, pw, k in ((0x1600, 0x1600, 0x1800, 0x1900, 5), (0x1A00, 0x1A00, 0x1B00, 0x1C00, 12)):
        o = k
        lanes += ('store(%d,0x1700);store(%d,c[%d]);store(%d,c[%d]?%d:0);store(%d,c[%d]);store(%d,c[%d]?%d:0);store(%d,c[%d]);'
                  'store(%d,c[%d]);' % (lw, lw + 4, o + 4, lw + 8, o, ow, ow + 4, o + 1, ow + 8, o + 2, pw, pw + 4, o + 3, lw + 0x40, o + 5))
    setup = ('memset(objects,0,sizeof(objects));K=c;'
             'store(0x1000+16,0x1100);store(0x1100+424,c[18]);store(0x1100+452,0x1400);objects[0x1100-0x1000+500]=c[2];'
             'store(0x1080,0x1400);store(0x1084,0x10000);store(0x1088,0x1100);'
             'store(0x1400,0x1300);store(0x1300+76,0x9100);store(0x1300+80,0x9104);'
             'store(0x1400+188,0x1600);store(0x1400+192,c[11]?0x1A00:0);store(0x1400+208,c[19]);store(0x1700+68,0x9108);' + lanes +
             'objects[0x10000-0x1000+0x64251]=c[0];store(0x10000+0x37350,c[22]);store(0x10000+0x37358,c[23]);'
             'store(0x10000+0x38970,c[26]?0x2A00:0);'
             'store(0x1F00+32,c[20]);store(0x1F00+40,c[21]);store(0x1F80,0x1F90);store(0x1F90+8,0x910C);'
             'store(0x1FC0,c[27]);')
    stubs = {
        'sub_' + lanepick: 'ctx.r3.u32=0;',
        'sub_' + net: 'ctx.r3.u32=0;',
        'sub_' + remote: 'ctx.r3.u32=K[1];',
        'sub_' + block: 'ctx.r3.u32=0x1F00;',
        'sub_' + into: 'uint32_t j=ctx.r3.u32==0x1600?K[24]:K[25];ctx.r3.u32=j==0?0:j==1?0x2A00:0x2B00;',
        'sub_' + vel: 'ctx.r3.u32=0x1F80;',
        'sub_' + left: 'ctx.f1.f64=getf(0x1FD0);',
    }
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'for(auto&c:C){' + setup + 'store(0x1FD0,c[28]);'
            'Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=0x1080;ctx.r5.u32=0x1E40;ctx.r6.u32=0x1E00;ctx.r7.u32=0x1E04;'
            'sub_%s(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t rb;memcpy(&rb,&r,4);' % want +
            ('printf("%%08x %%08x %%08x %%08x %%02x%s",rb,load(0x1E44),load(0x1E48),load(0x1E4C),objects[0x1E40-0x1000+16]);}' % NL) + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + want], main_src, a.out_dir, 'race_hold', stubs=stubs, scratch_end=0x80000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'race_hold_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] == '00000000' for o in out), 'with the wanted speed 0,',
          sum(int(o.split()[4], 16) & 0x80 != 0 for o in out), 'with entry 2')


if __name__ == '__main__':
    main()
