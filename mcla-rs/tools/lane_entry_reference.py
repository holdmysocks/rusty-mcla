"""An ambient car's slot 10 (F-93055) run offline: what becomes of its lane entries this frame. The unchanged generated code
on flat guest memory, with its "on a lane" test F-96472 in the closure and the active test F-17857 a stub giving the case's
answer; the obstacle record's slots 11 (+44, the lane update F-58331) and 14 (+56, every entry dropped F-11956) are logged
by the indirect-call dispatch. Inputs: the obstacle record there or not, active, the type's proxy (+108) and trailer (+112)
bytes, the velocity +208 (around 6 mph, 0 / -0, denormal, NaN, inf), car +500 (bit 0x10 and others), a behaviour or not,
its rail +188 or not and the rail's +4 word (bits 0x800 / 0x1000 and others).
Writes <out-dir>/lane_entry_ref.txt, one case per line: the inputs (ints, then the velocity as hex bits) "->" the calls
in order (A = slot 11, R = slot 14, - = none)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic const uint32_t*cur;static char calls[8];static int ncalls;\n')


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
    fn, active, on_lane = (addr(a.tool, f) for f in ('F-93055', 'F-17857', 'F-96472'))
    rnd = random.Random(61)
    thr = struct.unpack('>f', bytes.fromhex('40e6389e'))[0]
    cases = []
    NI = 9  # (ints: record, active, proxy, trailer, +500, behaviour, rail, rail +4, the slot 11 argument)
    for _ in range(4000):
        rec = 1 if rnd.random() < 0.97 else 0
        act = rnd.random() < 0.5
        proxy = rnd.choice((0, 0, 0, 1, 0x80))
        trailer = rnd.choice((0, 0, 0, 1, 2))
        b500 = rnd.choice((0, 0x10, 0x20, 0x30, 0x08, 0xFF, 0xEF, rnd.randrange(256)))
        beh = rnd.random() < 0.7
        rail = beh and rnd.random() < 0.85
        flags = rnd.choice((0, 0x800, 0x1000, 0x1800, 0x400, 0x2000, 0x27FF, 0xFFFFE7FF, 0xFFFFFFFF, rnd.getrandbits(32)))
        arg = rnd.getrandbits(32) & 0xFFFFFF  # (below 2^24: no test data word reads as an address)
        kind = rnd.random()
        if kind < 0.3:
            # (around the threshold: the squared speed v2 = fma(z, z, fma(x, x, y y)) on either side)
            s = (thr ** 0.5) * rnd.choice((1.0, 1 - 1e-7, 1 + 1e-7, 0.999, 1.001))
            d = [rnd.gauss(0, 1) for _ in range(3)]
            n = sum(x * x for x in d) ** 0.5
            v = [x / n * s for x in d]
        elif kind < 0.4:
            v = [rnd.choice((0.0, -0.0, 1e-40, -1e-40, float('nan'), float('inf'), -float('inf'), 1e30)) for _ in range(3)]
        elif kind < 0.7:
            v = [rnd.uniform(-3, 3) for _ in range(3)]
        else:
            v = [rnd.uniform(-40, 40), rnd.uniform(-5, 5), rnd.uniform(-40, 40)]
        cases.append([rec, int(act), proxy, trailer, b500, int(beh), int(rail), flags, arg] + v)
    nl = chr(10)
    NL = chr(92) + 'n'
    NF = len(cases[0])
    rows = nl.join('{%s},' % ','.join('0x%08x' % (x if i < NI else fb(x)) for i, x in enumerate(c)) for c in cases)
    s_add, s_rem = 0x82FF0100, 0x82FF0200
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x%x){calls[ncalls++]=(ctx.r4.u32==0x2000&&ctx.r5.u32==cur[8]&&ctx.r6.u32==0)?%s:%s;}'
            'else if(a==0x%x){calls[ncalls++]=ctx.r3.u32==0x2000?%s:%s;}'
            'else throw std::runtime_error("unexpected indirect call");}' % (s_add, "'A'", "'a'", s_rem, "'R'", "'r'"))
    # car 0x1000: +448 type 0x2200 (+108 proxy, +112 trailer), +452 behaviour 0x1E40 (+188 rail 0x1F40, its +4 the flags),
    # +496 the record 0x2000 (vtable 0x2100: +44 slot 11, +56 slot 14), +500 the byte, +208 .. +216 the velocity
    setup = ('memset(objects,0,0x2000);'
             'store(0x1000+448,0x2200);*pointer(0x2200+108)=c[2];*pointer(0x2200+112)=c[3];'
             'store(0x1000+452,c[5]?0x1E40:0);store(0x1E40+188,c[6]?0x1F40:0);store(0x1F40+4,c[7]);'
             'store(0x1000+496,c[0]?0x2000:0);store(0x2000,0x2100);store(0x2100+44,0x%x);store(0x2100+56,0x%x);' % (s_add, s_rem) +
             '*pointer(0x1000+500)=c[4];store(0x1000+208,c[9]);store(0x1000+212,c[10]);store(0x1000+216,c[11]);')
    body = ('static const uint32_t C[][%d]={' % NF + nl + rows + nl + '};' + nl +
            'for(auto&c:C){cur=c;ncalls=0;' + setup + 'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=c[8];ctx.r1.u32=0x2F00;'
            'sub_%s(ctx,nullptr);' % fn +
            'if(!ncalls)printf("-");for(int k=0;k<ncalls;k++)printf("%c",calls[k]);' +
            ('printf("%s");}' % NL) + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    stubs = {'sub_' + active: 'ctx.r3.u64=cur[1];'}
    exe = build(idx, ['sub_' + fn, 'sub_' + on_lane], main_src, a.out_dir, 'lane_entry', stubs=stubs, vector=False, scratch_end=0x3000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join(('%d' % x) if i < NI else '%08x' % fb(x) for i, x in enumerate(c)), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'lane_entry_ref.txt').write_text(nl.join(lines) + nl)
    tally = {}
    for o in out:
        tally[o] = tally.get(o, 0) + 1
    print(len(lines), 'cases', tally)


if __name__ == '__main__':
    main()
