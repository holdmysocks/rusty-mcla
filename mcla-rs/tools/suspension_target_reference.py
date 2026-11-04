"""The suspension target (F-11915) run offline: unchanged generated code on flat guest memory. Its branch for a wheel on a
level contact (the object at wheel +8 with +544 == 2): r3 = the wheel (rows +16 / +32 / +48, the contact frame's
sideways +320 and forwards +352, the sink depth +448, the compression +496), r4 = the contact record (normal +32, depth +68).
The static image holds 0 at the 0.007 global it subtracts; the game's static initialiser copies it there at
start-up (from the constant at 0x82082810, see docs/reference/vehicle/gjkfix__p1__body_decode.md), and main does the same.
The sinks are the materials' depths (0, sand / grass 0.025 - 0.05, 0.1) and random ones.
Writes <out-dir>/suspension_target_ref.txt: "row0 row1 row2 side fwd normal depth comp sink -> f1" as hex float bits."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        l = sum(x * x for x in v) ** 0.5
        if l > 0.1:
            return [x / l for x in v]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--fid', required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ad = addr(a.tool, a.fid)
    rnd = random.Random(36)
    cases = []
    for k in range(4000):
        # (rows near the car's own about a near-level contact, and wholly random ones for the clamps)
        if k % 4:
            tilt = [rnd.gauss(0, 0.15) for _ in range(3)]
            rows = [[1 + tilt[0], tilt[1] * 0.3, tilt[2] * 0.3], [tilt[1], 1, tilt[2]], [tilt[2] * 0.3, -tilt[0] * 0.3, 1]]
            n = [rnd.gauss(0, 0.2), 1.0, rnd.gauss(0, 0.2)]
            side, fwd = [1, rnd.gauss(0, 0.1), 0], [0, rnd.gauss(0, 0.1), 1]
        else:
            rows = [unit(rnd) for _ in range(3)]
            n, side, fwd = unit(rnd), unit(rnd), unit(rnd)
        depth = rnd.choice((rnd.uniform(-0.02, 0.15), rnd.uniform(0, 0.01), 0.0))
        comp = rnd.uniform(-0.2, 0.15)
        sink = rnd.choice((0.0, 0.025, 0.05, 0.05, 0.1, rnd.uniform(0, 0.2)))
        cases.append(rows + [side, fwd, n, [depth, comp, sink]])
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = lambda c: [fb(x) for v in c for x in v]
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in flat(c)) for c in cases)
    # wheel at 0x1000, the object at its +8 at 0x1400 (+544 = 2), the contact record at 0x1800
    offs = [16, 20, 24, 32, 36, 40, 48, 52, 56, 320, 324, 328, 352, 356, 360]
    stores = ''.join('store(0x%x,c[%d]);' % (0x1000 + o, i) for i, o in enumerate(offs))
    stores += 'store(0x1800+32,c[15]);store(0x1800+36,c[16]);store(0x1800+40,c[17]);store(0x1800+68,c[18]);store(0x1000+496,c[19]);store(0x1000+448,c[20]);'
    body = ('static const uint32_t C[][21]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x828FF840,load(0x82082810));store(0x1008,0x1400);store(0x1400+544,2);' + stores +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r1.u32=0x1F00;' +
            ('sub_%s(ctx,nullptr);{float r=(float)ctx.f1.f64;uint32_t u;memcpy(&u,&r,4);printf("%%08x%s",u);}}' % (ad, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.out_dir, 'suspension_target', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in flat(c)), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'suspension_target_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
