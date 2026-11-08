"""Run the original's car vfunc 120 (F-30960) offline with no behaviour (car +452 = 0), its no-rail exit: +416 = 0,
+412 = |car +208| (fmuls y y, fmadds x, fmadds z, fsqrts), the asked acceleration 0 in f1. Prints one case a line,
the words of +208 x y z -> +412 +416 f1 (as single), for the test traffic::measured_speed_tests.

    python tools/measured_speed_reference.py --recomp D:/MCLA-static --image <image> --tool <mcla-tool> > crates/game/src/testdata/measured_speed_ref.txt
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def word(rng):
    k = rng.random()
    if k < 0.05:
        return rng.choice([0, 0x80000000, 0x00000001, 0x007FFFFF, 0x80400000, 0x3F800000, 0x7F7FFFFF])
    if k < 0.15:
        return rng.getrandbits(32) & 0x807FFFFF  # denormals
    if k < 0.6:
        return struct.unpack('>I', struct.pack('>f', rng.uniform(-0.3, 0.3)))[0]
    return struct.unpack('>I', struct.pack('>f', rng.uniform(-60.0, 60.0)))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, default=Path('work_measured'))
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--n', type=int, default=3000)
    a = ap.parse_args()
    idx = index(a.recomp)
    fn = addr(a.tool, 'F-30960')
    rng = random.Random(30960)
    cases = [[word(rng) for _ in range(3)] for _ in range(a.n)]
    arr = ','.join('0x%08Xu,0x%08Xu,0x%08Xu' % tuple(c) for c in cases)
    body = ('static const uint32_t cases[] = {%s};\n' % arr +
            'auto be = [](uint32_t a, uint32_t v){store(a, v);};\n'
            'auto rd = [](uint32_t a){return load(a);};\n'
            'for (size_t k = 0; k < sizeof(cases) / 4; k += 3) {\n'
            '  memset(objects, 0x5A, sizeof(objects)); be(0x1000 + 452, 0);\n'
            '  be(0x1000 + 208, cases[k]); be(0x1000 + 212, cases[k + 1]); be(0x1000 + 216, cases[k + 2]);\n'
            '  Context c{}; c.r3.u32 = 0x1000; c.r4.u32 = 0x3000; c.r1.u32 = 0x2800; c.f1.f64 = 1.0 / 30.0;\n'
            '  sub_%s(c, nullptr); float f = float(c.f1.f64); uint32_t fw; memcpy(&fw, &f, 4);\n'
            '  printf("%%08X %%08X %%08X -> %%08X %%08X %%08X\\n", cases[k], cases[k + 1], cases[k + 2], rd(0x1000 + 412), rd(0x1000 + 416), fw);\n'
            '}\n' % fn)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n' + body + '}\n')
    # (the rail path's callees, never reached with no behaviour: empty)
    stubs = [f'sub_{addr(a.tool, f)}' for f in ('F-65049', 'F-50349', 'F-49294', 'F-14900', 'F-54830')]
    exe = build(idx, [f'sub_{fn}'], main_src, a.out, 'measured_speed', stubs=stubs, vector=True)
    print('# +208 x y z -> +412 +416 f1 (F-30960 with no behaviour)')
    print(run(exe, a.image), end='')


if __name__ == '__main__':
    main()
