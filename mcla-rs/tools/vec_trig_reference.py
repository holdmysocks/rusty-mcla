"""The vector unit's cosine F-72452 and arc cosine F-49700 run offline: the generated code unchanged in vector mode,
four lanes of v1 in, v1 out (each lane on its own). Writes <out-dir>/vec_trig_ref.txt: "cos|acos x -> y" (hex bits)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import re
import recomp_closure

# (the SDK's own vector helpers the generated code calls, taken from its intrinsics.h as they are: the unsigned convert
# of vcuxwfp128, the saturating convert of vcfpsxws128; vslw needs simde's avx2)
SDK = Path('D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h')


def sdk_helpers(names):
    text = SDK.read_text()
    out = []
    for n in names:
        m = re.search(r'^inline [\w:]+ ' + n + r'\(.*?^}\n', text, re.M | re.S)
        assert m, n
        out.append(m.group(0))
    return '#include <simde/x86/avx2.h>\n#include <climits>\nnamespace rex { namespace ppc {\n' + ''.join(out) + '} }\n'


recomp_closure.EXTRA += sdk_helpers(['simde_mm_cvtepu32_ps_', 'simde_mm_vctsxs'])


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
    fc, fa = addr(a.tool, 'F-72452'), addr(a.tool, 'F-49700')
    rnd = random.Random(23)
    xs = {'cos': [], 'acos': []}
    for _ in range(8000):
        xs['cos'].append(rnd.choice((rnd.uniform(-7, 7), rnd.uniform(-200, 200), rnd.uniform(-1e-3, 1e-3))))
        xs['acos'].append(rnd.choice((rnd.uniform(-1, 1), rnd.uniform(0.99, 1.0), -rnd.uniform(0.99, 1.0), rnd.uniform(-1e-4, 1e-4))))
    xs['cos'] += [0.0, -0.0, math.pi, -math.pi, math.pi / 2, 2 * math.pi, 1e-30, 1e6]
    xs['acos'] += [0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1e-30, 0.9999999]
    nl = chr(10)
    NL = chr(92) + 'n'
    jobs = [(k, x) for k in ('cos', 'acos') for x in xs[k]]
    while len(jobs) % 4:
        jobs.append(('acos', 0.25))
    rows = nl.join('{%d,0x%08x,0x%08x,0x%08x,0x%08x},' % ((jobs[i][0] == 'acos',) + tuple(fb(jobs[i + k][1]) for k in range(4)))
                   for i in range(0, len(jobs), 4) if all(jobs[i + k][0] == jobs[i][0] for k in range(4)))
    groups = [i for i in range(0, len(jobs), 4) if all(jobs[i + k][0] == jobs[i][0] for k in range(4))]
    body = ('static const uint32_t C[][5]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){Context ctx{};ctx.r1.u32=0x2F00;for(int k=0;k<4;k++)ctx.v1.u32[k]=c[1+k];' +
            ('if(c[0])sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);' % (fa, fc)) +
            'printf("%08x %08x %08x %08x' + NL + '",ctx.v1.u32[0],ctx.v1.u32[1],ctx.v1.u32[2],ctx.v1.u32[3]);}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fc, 'sub_' + fa], main_src, a.out_dir, 'vec_trig', vector=True, flags=('-msse4.1',))
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(groups)
    lines = []
    for i, o in zip(groups, out):
        for k, y in enumerate(o.split()):
            lines.append('%s %08x -> %s' % (jobs[i + k][0], fb(jobs[i + k][1]), y))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'vec_trig_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
