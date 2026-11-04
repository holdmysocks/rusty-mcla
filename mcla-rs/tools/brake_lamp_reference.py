"""The traffic car's brake lamp flag (F-56024) run offline: unchanged generated code on flat guest memory, with the
behaviour's slot 18 (F-86955, the race path's bParked byte +164), the physics test (F-17857) and the obstacle's slot 2
(F-59415, its car's velocity +208) dispatched as the game's vtables would.
Car at 0x1000 (+412 speed, +416 target speed, +420 entry in charge, +452 behaviour, +500 byte 0 so the physics
instance is the one at +112, its id u16 +120), behaviour 0x1400 (vtable 0x1800, +432 obstacle, +468 path), path 0x1900,
obstacle 0x1A00 (vtable 0x1B00, +16 its car 0x1C00), physics level at 0x1E00 ([0x828DA464]; +112 the 8-byte entries
at 0x2000, word +4 & 3 == 0: active).
Only states the game can reach: entry 1 (the stop line) always has target speed 0 (F-30960 copies the chosen entry's
speed, and F-83573's stop entry writes 0.0).
Writes <out-dir>/brake_lamp_ref.txt: "beh path parked v target slot id flags obst ux uy uz -> lamp", floats as hex bits."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# the two vtable calls go through `dispatch` (defined with main)
# (the shim has no count register: r2, which this code never uses, stands in for it)
recomp_closure.EXTRA += '#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    lamp, slot18, phys, obst = (addr(a.tool, f) for f in ('F-56024', 'F-86955', 'F-17857', 'F-59415'))
    rnd = random.Random(23)
    edge = [0.0, -0.0, 1e-30, 0.22352, 6.7056, 6.705599, 6.7056005, 6.70561, 13.4, 29.06]
    cases = []
    for _ in range(4000):
        beh = rnd.random() < 0.95
        path = beh and rnd.random() < 0.3
        parked = rnd.choice((0, 0, 1, 2, 0x80)) if path else 0
        v = rnd.choice(edge) if rnd.random() < 0.35 else rnd.uniform(0, 35)
        slot = rnd.choice((-1, -1, 0, 0, 1, 1, 2, 3))
        target = 0.0 if slot == 1 or rnd.random() < 0.2 else rnd.uniform(0, 35)
        pid = 0xFFFF if rnd.random() < 0.6 else rnd.randrange(4)
        flags = rnd.randrange(16)
        ob = beh and rnd.random() < 0.8
        if ob and rnd.random() < 0.5:
            # near the 1.5 x boundary: |u| about v / 1.5
            d = [rnd.uniform(-1, 1) for _ in range(3)]
            n = sum(x * x for x in d) ** 0.5 or 1.0
            s = v / 1.5 * (1 + rnd.choice((0.0, 1e-7, -1e-7, 1e-6, -1e-6, 1e-3)))
            u = [x / n * s for x in d]
        else:
            u = [rnd.uniform(-20, 20) * rnd.choice((0, 1, 1)) for _ in range(3)]
        cases.append((int(beh), int(path), parked, v, target, slot, pid, flags, int(ob), u))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in
                   [c[0], c[1], c[2], fb(c[3]), fb(c[4]), c[5] & 0xFFFFFFFF, c[6], c[7], c[8]] + [fb(x) for x in c[9]]) for c in cases)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0x%s)sub_%s(ctx,base);else if(a==0x%s)sub_%s(ctx,base);'
            'else throw std::runtime_error("unexpected indirect call");}' % (slot18, slot18, obst, obst))
    body = ('static const uint32_t C[][12]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'store(0x1000+412,c[3]);store(0x1000+416,c[4]);store(0x1000+420,c[5]);store(0x1000+452,c[0]?0x1400:0);' +
            'pointer(0x1000+120)[0]=c[6]>>8;pointer(0x1000+120)[1]=c[6];' +
            'store(0x1400,0x1800);store(0x1800+72,0x%s);store(0x1400+468,c[1]?0x1900:0);pointer(0x1900+164)[0]=c[2];' % slot18 +
            'store(0x1400+432,c[8]?0x1A00:0);store(0x1A00,0x1B00);store(0x1B00+8,0x%s);store(0x1A00+16,0x1C00);' % obst +
            'store(0x1C00+208,c[9]);store(0x1C00+212,c[10]);store(0x1C00+216,c[11]);' +
            'store(0x828DA464,0x1E00);store(0x1E00+112,0x2000);if(c[6]<4)store(0x2000+8*c[6]+4,c[7]);' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2F00;' +
            ('sub_%s(ctx,nullptr);printf("%%u%s",ctx.r3.u32&0xFF);}' % (lamp, NL)) + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + lamp, 'sub_' + slot18, 'sub_' + obst, 'sub_' + phys],
                main_src,
                a.out_dir, 'brake_lamp')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%d %d %d %08x %08x %d %d %d %d %s -> %s' % (c[0], c[1], c[2], fb(c[3]), fb(c[4]), c[5], c[6], c[7], c[8],
                                                         ' '.join('%08x' % fb(x) for x in c[9]), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'brake_lamp_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o == '1' for o in out), 'lit')


if __name__ == '__main__':
    main()
