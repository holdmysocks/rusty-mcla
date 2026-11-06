"""The walker's wait timer F-84843 run offline: unchanged generated code on flat guest memory (copy of the pattern of
clamp_length_reference.py). The walker manager M (r3) holds the ped at +224; the ped holds its MWC (+384 lo, +388 carry) and
M flags (+260, the stop count in bits 3 and up, 0x4 / 0x2 cleared by the call); the idle clip's length v is read through the
ped's chain +456 -> +20 -> +64 -> +28 -> float (the ped type's idle clip). Per case: v, MWC lo, carry, flags.
Writes <out-dir>/stop_time_ref.txt, one case a line: "v lo carry flags -> timer lo carry flags" as hex words."""
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
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    ad = addr(a.tool, 'F-84843')
    rnd = random.Random(84843)
    cases = []
    lens = [0.0, 0.0, 1.0, 2.333, 4.999, 5.0, 5.0001, 25.667, -1.0, 1e-6, 1e30, float('nan')]
    for _ in range(a.cases):
        k = rnd.random()
        if k < 0.3:
            v = rnd.choice(lens)
        elif k < 0.7:
            v = rnd.uniform(0.0, 5.0)
        else:
            v = rnd.uniform(0.0, 40.0)
        cases.append([fb(v), rnd.getrandbits(32), rnd.getrandbits(32), rnd.choice((0, 8, 6, 0x2F, rnd.getrandbits(32)))])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    body = ('static const uint32_t C[][4]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'store(0x1000+224,0x1100);store(0x1100+456,0x1200);store(0x1200+20,0x1300);store(0x1300+64,0x1400);store(0x1400+28,0x1500);' +
            'store(0x1500,c[0]);store(0x1100+384,c[1]);store(0x1100+388,c[2]);store(0x1000+260,c[3]);' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2F00;' +
            ('sub_%s(ctx,nullptr);printf("%%08x %%08x %%08x %%08x%s",load(0x1000+228),load(0x1100+384),load(0x1100+388),load(0x1000+260));}' % (ad, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.out_dir, 'stop_time', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that would read as addresses of the original are split by '_'; the test removes it)
    (a.out_dir / 'stop_time_ref.txt').write_text(re.sub(r'(82[0-9a-f]{2})([0-9a-f]{4})', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
