"""The lamp ease F-24995 and the slipstream trail's fade F-38786 run offline: the generated code unchanged on a state
record at 0x1000 (+0 rate, +4 target: a byte for the ease, a float for the fade, +8 value) with the tick written into the
image's global (lis -32131 / 29952 + 8). Writes <out-dir>/fades_ref.txt: "ease|fade rate target value dt -> value"
(hex bits; the ease's target 0 or 1)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

TICK = ((-32131 << 16) + 29952 + 8) & 0xFFFFFFFF


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
    fe, ff = addr(a.tool, 'F-24995'), addr(a.tool, 'F-38786')
    rnd = random.Random(41)
    sp = (0.0, -0.0, float('nan'), float('inf'), 1.0, 0.99, 0.01)
    cases = []
    for _ in range(6000):
        k = rnd.choice(('ease', 'fade'))
        rate = rnd.choice((9.0, 18.0, 4.5, rnd.uniform(0, 30)))
        dt = rnd.choice((1 / 60, 1 / 30, rnd.uniform(0, 0.1)))
        tgt = float(rnd.randrange(2)) if k == 'ease' else rnd.choice((0.0, 1.0, rnd.uniform(0, 1)))
        v = rnd.choice((rnd.uniform(0, 1), rnd.uniform(0.97, 1.0), rnd.uniform(0, 0.03), tgt + rnd.uniform(-0.02, 0.02)))
        c = [rate, tgt, v, dt]
        for i in (0, 2, 3) if k == 'ease' else (0, 1, 2, 3):
            if rnd.random() < 0.03:
                c[i] = rnd.choice(sp)
        cases.append((k, c))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%d,0x%08x,0x%08x,0x%08x,0x%08x},' % ((k == 'fade', fb(c[0]), int(c[1]) if k == 'ease' else fb(c[1]), fb(c[2]), fb(c[3]))) for k, c in cases)
    body = ('static const uint32_t C[][5]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000,c[1]);if(c[0])store(0x1004,c[2]);else objects[4]=uint8_t(c[2]);' +
            ('store(0x1008,c[3]);store(0x%08X,c[4]);Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' % TICK) +
            ('if(c[0])sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);printf("%%08x%s",load(0x1008));}' % (ff, fe, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fe, 'sub_' + ff], main_src, a.out_dir, 'fades')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'fades_ref.txt').write_text(nl.join('%s %08x %s %08x %08x -> %s' % (k, fb(c[0]), ('%d' % c[1]) if k == 'ease' else '%08x' % fb(c[1]), fb(c[2]), fb(c[3]), o) for (k, c), o in zip(cases, out)) + nl)
    print(len(cases), 'cases')


if __name__ == '__main__':
    main()
