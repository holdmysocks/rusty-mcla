"""A wheel's compression held in its travel F-92300 and the axle's displacement F-48387 run offline: the generated
code unchanged (wheel at 0x1000: +4 the tune at 0x1800 with TravelLimit +72 and TravelExtent +76, +432 the offset, +496
the compression). Writes <out-dir>/wheel_comp_ref.txt: "comp extent limit offset -> clamp disp" (hex float bits)."""
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


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fc, fd = addr(a.tool, 'F-92300'), addr(a.tool, 'F-48387')
    rnd = random.Random(31)
    sp = (0.0, -0.0, float('nan'), float('inf'), -float('inf'))
    cases = []
    for _ in range(5000):
        ext, lim = rnd.uniform(0, 0.3), rnd.uniform(0, 0.3)
        if rnd.random() < 0.1:
            ext, lim = -lim - 0.01, rnd.uniform(-0.2, 0.1)
        c = [rnd.choice((rnd.uniform(-0.5, 0.5), lim, -ext)), ext, lim, rnd.uniform(-0.1, 0.1)]
        for k in range(4):
            if rnd.random() < 0.04:
                c[k] = rnd.choice(sp)
        cases.append(c)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in cases)
    out_f = 'float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);'
    body = ('static const uint32_t C[][4]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1004,0x1800);store(0x1000+496,c[0]);store(0x1800+76,c[1]);store(0x1800+72,c[2]);store(0x1000+432,c[3]);' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);{%sprintf("%%08x ",u);}' % (fc, out_f)) +
            ('ctx=Context{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);{%sprintf("%%08x%s",u);}}' % (fd, out_f, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fc, 'sub_' + fd], main_src, a.out_dir, 'wheel_comp')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'wheel_comp_ref.txt').write_text(nl.join('%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cases, out)) + nl)
    print(len(cases), 'cases')


if __name__ == '__main__':
    main()
