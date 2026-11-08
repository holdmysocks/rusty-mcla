"""The script native's arccosine F-62232 run offline with what it calls (the C runtime's acos F-68873 -> F-11618; generated
code unchanged, scalar mode): r3 the native's call record (+8 -> the argument float, +0 -> where the result goes).

Cases: random arguments in [-1.2, 1.2], 0.5 and +-1 with one or two ulps either side, +-0, tiny ones, NaN and the
infinities.

|x| > 1 gives NaN; its sign bit is the x86 recomp's (fsqrt's default NaN, then fneg), not the Xbox's, so the test compares
NaN as NaN.

Writes <out-dir>/acos_ref.txt, one case a line: "x -> result" (float bits, hex)."""
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
    fn = addr(a.tool, 'F-62232')
    rnd = random.Random(11102)
    ws = set()
    for e in (0.5, 1.0, 0.0):
        for s in (1, -1):
            b = fb(s * e)
            ws.update(b + d for d in (-2, -1, 0, 1, 2) if 0 <= b + d < 2 ** 32)
    ws.update((0x7fc00000, 0x7f800000, 0xff800000, 0x00000001, 0x80000001, 0x3f7fffff, 0xbf7fffff))
    ws = sorted(ws)
    while len(ws) < 8000:
        ws.append(fb(rnd.choice((rnd.uniform(-1.2, 1.2), rnd.uniform(-1, 1), rnd.uniform(0.45, 0.55) * rnd.choice((1, -1)),
                                 rnd.choice((1, -1)) * 10 ** rnd.uniform(-30, 0)))))
    nl = chr(10)
    NL = chr(92) + 'n'
    body = ('static const uint32_t C[]={' + ','.join('0x%08x' % w for w in ws) + '};' + nl +
            'for(auto c:C){memset(objects,0,sizeof(objects));store(0x1000,0x1100);store(0x1008,0x1200);store(0x1200,c);' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);printf("%%08x' % fn) + NL + '",load(0x1100));}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'acos')
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(ws), (len(out), len(ws))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'acos_ref.txt').write_text(nl.join('%08x -> %s' % (w, o) for w, o in zip(ws, out)) + nl)
    print(len(ws), 'cases')


if __name__ == '__main__':
    main()
