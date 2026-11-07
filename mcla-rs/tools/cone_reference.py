"""The contact rows' friction cone F-48120 run offline (generated code unchanged, vector mode, the recomp's own
-msse4.1 -mssse3): r3 the row (+3856 the normal row c, +3872 mu in every lane), r4 the multipliers x, in and out.

Writes <out-dir>/cone_ref.txt, one case a line: "x0 x1 x2 c0 c1 c2 mu -> y0 y1 y2" (hex float bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import re
import recomp_closure

# (the SDK's own unsigned convert the generated code calls for vcuxwfp, taken from its intrinsics.h as it is)
SDK = Path('D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h')
_m = re.search(r'^inline [\w:]+ simde_mm_cvtepu32_ps_\(.*?^}\n', SDK.read_text(), re.M | re.S)
recomp_closure.EXTRA += '#include <simde/x86/avx2.h>\n#include <climits>\nnamespace rex { namespace ppc {\n' + _m.group(0) + '} }\n'


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
    fc = addr(a.tool, 'F-48120')
    rnd = random.Random(61)
    cases = []
    for i in range(6000):
        s = 10 ** rnd.uniform(-4, 4)
        x = [rnd.uniform(-1, 1) * s for _ in range(3)]
        if i % 7 == 0:
            x[1] = x[2] = 0.0
        if i % 11 == 0:
            x[0] = 0.0
        c = [1.0, rnd.uniform(-0.5, 0.5), rnd.uniform(-0.5, 0.5)] if rnd.random() < 0.8 else [rnd.uniform(-2, 2) for _ in range(3)]
        mu = rnd.choice((0.0, 0.5, 1.0, 1.2, rnd.uniform(0, 2)))
        cases.append(x + c + [mu])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(v) for v in c) for c in cases)
    body = ('static const uint32_t C[][7]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'for(int k=0;k<3;k++){store(0x2000+4*k,c[k]);store(0x1000+3856+4*k,c[3+k]);}' +
            'for(int k=0;k<4;k++)store(0x1000+3872+4*k,c[6]);' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x2000;sub_%s(ctx,nullptr);' % fc) +
            'printf("%08x %08x %08x' + NL + '",load(0x2000),load(0x2004),load(0x2008));}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fc], main_src, a.out_dir, 'cone', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % fb(v) for v in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'cone_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
