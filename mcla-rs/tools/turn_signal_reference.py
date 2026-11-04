"""The traffic car's turn signal flags (F-41231: bit 0x20, F-81289: bit 0x10) run offline: unchanged generated code on flat
guest memory, with the lane's end point (slot 13 F-93886 -> the lane's curve, slot 15 F-58682, and the curve's point getter F-48295,
dispatched as the game's vtables would) and the owner test F-29760. The car's position getter (F-58860, which reads the
thread's context) is replaced by a stub that returns the case's position: it is an input here, not part of the rule.
Car at 0x1000 (+3572 blink, +3580 offset, +452 behaviour), behaviour 0x1400 (+76 the follower's lane-change rail: 0, so
F-68395 is false as offline traffic never changes lanes; +188 the current lane 0x1600, +192 the next lane 0x1700 or 0,
+208 the phase), lanes with vtable 0x1800 and owners at 0x1900 / 0x1B00 (+4 flags, +16 word) or none, the current lane's
curve at +48 (vtable 0x1880, +28 last index, +32 the index table at 0x1A00), vertex pool at 0x1C00 ([0x828DA6B4]).
Writes <out-dir>/turn_signal_ref.txt: "blink on owner1 flags1 w10 phase next owner2 flags2 px py pz ex ey ez -> a b"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# (the shim has no count register: r2, which this code never uses, stands in for it; the vtable calls go through `dispatch`)
recomp_closure.EXTRA += '#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
# (nor a signed halfword view: the lane end's `extsh` reads it as the halfword's two's complement)
_extract = recomp_closure.extract
recomp_closure.extract = lambda p, n: _extract(p, n).replace('ctx.r9.s16', 'int16_t(ctx.r9.u16)')


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
    sa, sb, end, pos, curve, point = (addr(a.tool, f) for f in ('F-41231', 'F-81289', 'F-93886', 'F-58860', 'F-58682', 'F-48295'))
    rnd = random.Random(31)
    flagset = (0, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x1000, 0x1010, 0x1020)
    cases = []
    for _ in range(5000):
        on = 0.2 * rnd.random() - 0.1
        if rnd.random() < 0.3:
            blink = struct.unpack('>f', struct.pack('>f', on + 0.375))[0] * rnd.choice((1.0, 1.0000001, 0.9999999))
        else:
            blink = rnd.uniform(0, 0.85)
        o1 = rnd.random() < 0.9
        f1 = rnd.choice(flagset) if o1 else 0
        w10 = rnd.choice((0, 0, 1, 7)) if o1 else 0
        phase = rnd.choice((0.5, 0.49999997, 0.50000006, rnd.random()))
        nx = rnd.random() < 0.8
        o2 = nx and rnd.random() < 0.9
        f2 = rnd.choice(flagset) if o2 else 0
        e = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        d = [rnd.uniform(-1, 1) for _ in range(3)]
        n = sum(x * x for x in d) ** 0.5 or 1.0
        r = rnd.choice((100.0, 99.99, 100.01, 99.9999, rnd.uniform(0, 200)))
        p = [e[k] + d[k] / n * r for k in range(3)]
        cases.append((blink, on, int(o1), f1, w10, phase, int(nx), int(o2), f2, p, e))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in
                   [fb(c[0]), fb(c[1]), c[2], c[3], c[4], fb(c[5]), c[6], c[7], c[8]] + [fb(x) for x in c[9]] + [fb(x) for x in c[10]])
                   for c in cases)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0x%s)sub_%s(ctx,base);else if(a==0x%s)sub_%s(ctx,base);'
            'else if(a==0x%s)sub_%s(ctx,base);else throw std::runtime_error("unexpected indirect call");}' % (end, end, curve, curve, point, point))
    setup = ('memset(objects,0,sizeof(objects));store(0x1000+3572,c[0]);store(0x1000+3580,c[1]);store(0x1000+452,0x1400);'
             'store(0x1400+188,0x1600);store(0x1400+192,c[6]?0x1700:0);store(0x1400+208,c[5]);'
             'store(0x1600,0x1800);store(0x1700,0x1800);store(0x1800+52,0x%s);store(0x1800+60,0x%s);' % (end, curve) +
             'store(0x1600+8,c[2]?0x1900:0);store(0x1900+4,c[3]);store(0x1900+16,c[4]);store(0x1700+8,c[7]?0x1B00:0);store(0x1B00+4,c[8]);'
             'store(0x1600+48,0x1880);store(0x1880+8,0x%s);pointer(0x1600+48+28)[0]=0;pointer(0x1600+48+28)[1]=3;store(0x1600+48+32,0x1A00);' % point +
             'pointer(0x1A00+6)[0]=0;pointer(0x1A00+6)[1]=5;store(0x828DA6B4,0x1C00);store(0x1C00+80,c[12]);store(0x1C00+84,c[13]);store(0x1C00+88,c[14]);'
             'store(0x1E00,c[9]);store(0x1E04,c[10]);store(0x1E08,c[11]);')
    body = ('static const uint32_t C[][15]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){unsigned r[2];for(int k=0;k<2;k++){' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2F00;if(k==0)sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);r[k]=ctx.r3.u32&0xFF;}' % (sa, sb) +
            ('printf("%%u %%u%s",r[0],r[1]);}' % NL) + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + sa, 'sub_' + sb, 'sub_' + end, 'sub_' + curve, 'sub_' + point], main_src, a.out_dir, 'turn_signal',
                stubs={'sub_' + pos: 'ctx.r3.u32=0x1E00;'})
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%08x %08x %d %x %d %08x %d %d %x %s %s -> %s' % (fb(c[0]), fb(c[1]), c[2], c[3], c[4], fb(c[5]), c[6], c[7], c[8],
                                                            ' '.join('%08x' % fb(x) for x in c[9]), ' '.join('%08x' % fb(x) for x in c[10]), o)
             for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'turn_signal_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] == '1' for o in out), 'with 0x20,', sum(o.split()[1] == '1' for o in out), 'with 0x10')


if __name__ == '__main__':
    main()
