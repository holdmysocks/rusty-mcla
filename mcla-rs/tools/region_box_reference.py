"""The traffic region box getters F-81783 (length) and F-68633 (width) run offline: unchanged generated code on flat guest
memory. The tune record (r3) holds the along-box pairs (+36 / +40 length, +20 / +24 width) and the perpendicular pairs
(+60 / +64 length, +44 / +48 width); f1 = mph; the flag byte (r5) picks the perpendicular pair. The result is
lerp(pair, t) with t = clamp(mph x 0.016666668, 0, 1) through fsel, the lerp fused: fma(t, b - a, a). (The centre getter
F-97927 returns its vector through the stack and is not run here.)
Writes <out-dir>/region_box_ref.txt, one case a line: "which flag mph a b -> result" as hex words (which 0 length, 1 width)."""
import argparse
import random
import re
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
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    ids = ['F-81783', 'F-68633']
    ad = [addr(a.tool, f) for f in ids]
    rnd = random.Random(81783)
    cases = []
    for _ in range(a.cases):
        which = rnd.randrange(2)
        mph = rnd.choice((0.0, 60.0, 30.0, 59.99, 120.0, -5.0, rnd.uniform(0, 80), rnd.uniform(0, 80)))
        lo, hi = rnd.uniform(5, 300), rnd.uniform(5, 300)
        cases.append((which, rnd.randrange(2), mph, lo, hi))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%d,%d,0x%08x,0x%08x,0x%08x},' % (w, f, fb(m), fb(l), fb(h)) for w, f, m, l, h in cases)
    # the record: the along pair and the perpendicular pair of the getter's offsets get the case's pair, the other pairs other words
    off = [(36, 40, 60, 64), (20, 24, 44, 48), (28, 32, 52, 56)]
    body = ('static const uint32_t C[][5]={' + nl + rows + nl + '};' + nl +
            'static const int OFF[3][4]={{36,40,60,64},{20,24,44,48},{28,32,52,56}};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));int w=int(c[0]);' +
            'for(int k=0;k<4;k++)store(0x1000+OFF[w][k],0x3F800000+k);' +
            'int pa=c[1]?2:0;store(0x1000+OFF[w][pa],c[3]);store(0x1000+OFF[w][pa+1],c[4]);' +
            'store(0x1000+OFF[w][c[1]?0:2],0x40000000);store(0x1000+OFF[w][c[1]?1:3],0x40400000);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1000;ctx.r5.u32=c[1];ctx.r6.u32=c[1];' +
            '{float m;uint32_t u=c[2];memcpy(&m,&u,4);ctx.f1.f64=m;}' +
            'switch(w){case 0:sub_%s(ctx,nullptr);break;default:sub_%s(ctx,nullptr);}' % tuple(ad) +
            ('{float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);printf("%%08x%s",u);}}' % NL) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + x for x in ad], main_src, a.out_dir, 'region_box', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%d %d %08x %08x %08x -> %s' % (w, f, fb(m), fb(l), fb(h), o) for (w, f, m, l, h), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that would read as addresses of the original are split by '_'; the test removes it)
    (a.out_dir / 'region_box_ref.txt').write_text(re.sub(r'(82[0-9a-f]{2})([0-9a-f]{4})', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
