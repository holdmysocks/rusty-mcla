"""Run the original's constraint slot update F-49294 offline with its needed acceleration F-20809 stubbed (it returns the
case's a in f1), on random slots, entries and thresholds. Prints one case a line - slot on, entry reset, obstacle (0 none,
1 a car, 9 the player), the words of a, the entry's distance and margin, and the six thresholds (decel band, MinDecel,
MaxDecel, accel band, MinAccel, MaxAccel + bonus) -> the slot's flags byte, +0 (a) and +4 (gap) - for the test
traffic::slot_update_tests.

    python tools/slot_update_reference.py --recomp D:/MCLA-static --image <image> --tool <mcla-tool> > crates/game/src/testdata/slot_update_ref.txt
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import memcap

memcap.apply()


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fw(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def odd(rng):
    return rng.choice([0, 0x80000000, 0x00000001, 0x807FFFFF, 0x7FC00000, 0x7F800000, 0xFF800000, 0x7F7FFFFF])


def case(rng):
    tune = [rng.choice([0.2, 1.0, rng.uniform(0, 2)]), rng.choice([3.0, 1.0, rng.uniform(0, 5)]), rng.choice([22.5, 67.5, 12.75, rng.uniform(1, 70)]),
            rng.choice([1.0, 0.2, rng.uniform(0, 2)]), rng.choice([1.0, 3.0, rng.uniform(0, 5)]), rng.choice([5.0, 3.0, rng.uniform(0, 8)])]
    th = [tune[4] + tune[3] * 0.5, tune[4] - tune[3] * 0.5, tune[1] + tune[0] * 0.5, tune[1] - tune[0] * 0.5]
    k = rng.random()
    if k < 0.3:   # near a threshold
        a = rng.choice([th[0], th[1], -th[2], -th[3], tune[5], -tune[2], -1.25 * tune[2]]) * (1 + rng.choice([-1, 0, 1]) * 1e-7)
    elif k < 0.4:
        a = rng.choice([0.0, -0.0])
    else:
        a = rng.uniform(-100, 12)
    dist, margin = rng.uniform(-5, 80), rng.choice([0.0, rng.uniform(0, 10)])
    if rng.random() < 0.1:
        margin = dist
    ws = [fw(a), fw(dist), fw(margin)] + [fw(t) for t in tune]
    if rng.random() < 0.06:
        ws[rng.randrange(len(ws))] = odd(rng)
    return [rng.randrange(2), int(rng.random() < 0.2), rng.choice([0, 1, 9])] + ws


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, default=Path('work_slot_update'))
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--n', type=int, default=4000)
    a = ap.parse_args()
    idx = index(a.recomp)
    fn, need = addr(a.tool, 'F-49294'), addr(a.tool, 'F-20809')
    rng = random.Random(49294)
    cases = [case(rng) for _ in range(a.n)]
    arr = ','.join(','.join('0x%08Xu' % w for w in c) for c in cases)
    nl = chr(92) + 'n'
    body = ('static const uint32_t cases[] = {%s};\n' % arr +
            'auto be = [](uint32_t a, uint32_t v){store(a, v);};\n'
            'auto rd = [](uint32_t a){return load(a);};\n'
            'for (size_t k = 0; k < sizeof(cases) / 4; k += 12) {\n'
            '  const uint32_t *q = cases + k; memset(objects, 0, sizeof(objects));\n'
            '  // slot 0x1000 (+8 flags: 0x80 active, 0x40 on), entry 0x1100 (+0 obstacle, +4 dist, +12 margin, +16 flags),\n'
            '  // thresholds 0x1200 (+0 .. +20), the obstacle 0x1300 (+6 its type)\n'
            '  (*pointer(0x1000 + 8)) = q[0] ? 0x40 : 0; (*pointer(0x1100 + 16)) = 0x80 | (q[1] ? 0x40 : 0);\n'
            '  be(0x1100, q[2] ? 0x1300u : 0u); (*pointer(0x1300 + 6)) = (uint8_t)q[2];\n'
            '  be(0x1400, q[3]); be(0x1100 + 4, q[4]); be(0x1100 + 12, q[5]);\n'
            '  for (int j = 0; j < 6; j++) be(0x1200 + 4 * j, q[6 + j]);\n'
            '  Context c{}; c.r3.u32 = 0x1000; c.r4.u32 = 0x1100; c.r5.u32 = 0x1200; c.r1.u32 = 0x2800;\n'
            '  sub_%s(c, nullptr);\n'
            '  for (int j = 0; j < 12; j++) printf("%%08X ", q[j]);\n'
            '  printf("-> %%02X %%08X %%08X' % fn + nl + '", (*pointer(0x1000 + 8)), rd(0x1000), rd(0x1004));\n'
            '}\n')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n' + body + '}\n')
    stub = {f'sub_{need}': 'uint32_t w = load(0x1400); float f; memcpy(&f, &w, 4); ctx.f1.f64 = f;'}
    exe = build(idx, [f'sub_{fn}'], main_src, a.out, 'slot_update', stubs=stub)
    print('# on reset obstacle a dist margin band_d min_d max_d band_a min_a up -> flags +0 +4 (F-49294)')
    print(run(exe, a.image), end='')


if __name__ == '__main__':
    main()
