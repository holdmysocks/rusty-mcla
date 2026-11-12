"""Run the original's speed controller F-30960 offline from its slot updates on: the four constraint entries through
F-49294 (unchanged), the pick, free driving F-54830 (unchanged), the start delay. Stubbed: the needed acceleration
F-20809 (it returns the case's a for the entry it is given), the heading F-50349 (only F-20809 reads it), the player
lookup F-65049 (it returns a fake player car whose rail and heading the case sets, for the x 3 MaxDecel test), and
F-14900 (never reached: the timer +404 is 0). Prints one case a line for the test traffic::controller_pick_tests:

  <inputs as hex words> -> <f1> <+420> <+440> <the four slots' flags> <the four slots' +0> <the four slots' +4>

    python tools/controller_pick_reference.py --recomp D:/MCLA-static --image <image> --tool <mcla-tool> > crates/game/src/testdata/controller_pick_ref.txt

Input words: per entry (4 x 7): active, reset, player (entry 0 only), a, dist, margin, target; per slot (4): on; then
band_a min_a max_a band_d min_d max_d bonus speed(+412) want(+152) waited(+440) wait(+444) dt, across (0 / 1: the player
on our rail with |dot| < 0.7071, 2: on our rail but along it).
"""
import argparse
import re
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import memcap

memcap.apply()

NE = 7


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fw(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def case(rng):
    tune = [rng.choice([1.0, rng.uniform(0, 2)]), rng.choice([1.0, rng.uniform(0, 3)]), rng.choice([5.0, 3.0, rng.uniform(1, 8)]),
            rng.choice([0.2, rng.uniform(0, 2)]), rng.choice([3.0, rng.uniform(0, 5)]), rng.choice([22.5, 12.75, rng.uniform(1, 30)])]
    bonus = rng.uniform(-0.5, 0.5)
    speed = rng.choice([0.0, 0.0, rng.uniform(0, 30)])
    want = rng.choice([speed, rng.uniform(0, 30)])
    ws = []
    for k in range(4):
        active = int(rng.random() < 0.6)
        a = rng.choice([0.0, -0.0, rng.uniform(-30, 8), rng.uniform(-1, 1), tune[2] + bonus, -tune[5], rng.uniform(1.5, 4)])
        if rng.random() < 0.03:
            a = rng.choice([float('nan'), float('inf'), -float('inf')])
        dist = rng.uniform(-3, 60)
        margin = rng.choice([0.0, rng.uniform(0, 8)]) if k != 2 and k != 3 else 0.0
        ws += [active, int(rng.random() < 0.15), int(k == 0 and rng.random() < 0.25), fw(a), fw(dist), fw(margin), fw(rng.uniform(0, 30))]
    ws += [int(rng.random() < 0.5) for _ in range(4)]
    waited, wait = rng.choice([0.0, rng.uniform(0, 1)]), rng.uniform(0, 0.8)
    ws += [fw(x) for x in tune + [bonus, speed, want, waited, wait, rng.choice([1 / 30, rng.uniform(0.01, 0.1)])]]
    ws += [rng.choice([0, 0, 1, 2])]
    return ws


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, default=Path('work_controller_pick'))
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--n', type=int, default=4000)
    ap.add_argument('--quiet', action='store_true', help='the cases of a car whose race path has bParked: F-33409 skips '
                    '(behaviour slot 18, its context byte +16 bit 0x8), so the context is as F-39469 leaves it - no entry '
                    'active, +152 want 0, +156 0 (the on bits and the rest as drawn)')
    a = ap.parse_args()
    idx = index(a.recomp)
    fn = addr(a.tool, 'F-30960')
    rng = random.Random(30960)
    cases = [case(rng) for _ in range(a.n)]
    if a.quiet:
        for c in cases:
            for e in range(4):
                c[7 * e] = 0
            c[32 + 8] = 0
    div = '0u' if a.quiet else '0x3F800000u'
    width = len(cases[0])
    arr = ','.join(','.join('0x%08Xu' % w for w in c) for c in cases)
    nl = chr(92) + 'n'
    # car 0x1000 (slots +60, stride 12), controller 0x1200 (entries +64, stride 20; +152 want, +156 1.0), a per entry at
    # 0x1500, type tune 0x1600, behaviour 0x1700 (+188 / +192 the rails 0x1780 / 0x17A0), obstacles 0x1900 + 16k,
    # the player 0x1C00 (+640 its rail, +48 -> 0x1C80 whose +28 -> 0x1CC0 (+16 +32 / +40 the heading x / z), +624 / +632
    # the other vector), the game object 0x1D00 (its +132 / +8 the player list), r13 -> 0x1E00 -> 0x1E40 (+72 the flag 0)
    body = ('static const uint32_t cases[] = {%s};\n' % arr +
            'auto be = [](uint32_t a, uint32_t v){store(a, v);};\n'
            'auto rd = [](uint32_t a){return load(a);};\n'
            'auto pf = [](uint32_t a, float v){putf(a, v);};\n'
            'for (size_t k = 0; k < sizeof(cases) / 4; k += %d) {\n' % width +
            '  const uint32_t *q = cases + k; memset(objects, 0, sizeof(objects));\n'
            '  for (int e = 0; e < 4; e++) { const uint32_t *x = q + 7 * e; uint32_t en = 0x1200 + 64 + 20 * e;\n'
            '    be(en, x[2] ? 0x1900u + 16 * e : 0u); (*pointer(0x1900 + 16 * e + 6)) = x[2] ? 9 : 1;\n'
            '    be(en + 4, x[4]); be(en + 8, x[6]); be(en + 12, x[5]); (*pointer(en + 16)) = (x[0] ? 0x80 : 0) | (x[1] ? 0x40 : 0);\n'
            '    be(0x1500 + 4 * e, x[3]); (*pointer(0x1000 + 60 + 12 * e + 8)) = q[28 + e] ? 0x40 : 0; }\n'
            '  const uint32_t *t = q + 32;\n'
            '  for (int j = 0; j < 6; j++) be(0x1600 + 128 + 4 * j, t[j]);\n'
            '  be(0x1000 + 428, t[6]); be(0x1000 + 412, t[7]); be(0x1200 + 152, t[8]); be(0x1200 + 156, DIV);\n'
            '  be(0x1000 + 440, t[9]); be(0x1000 + 444, t[10]); be(0x1000 + 448, 0x1600); be(0x1000 + 452, 0x1700);\n'
            '  be(0x1700 + 188, 0x1780); be(0x1700 + 192, 0x17A0);\n'
            '  uint32_t across = q[%d];\n' % (width - 1) +
            '  be(GAMEPTR, 0x1D00); be(0x1D00 + 132, across ? 1 : 0); be(0x1D00 + 8, across ? 1 : 0);\n'
            '  be(0x1E00, 0x1E40);\n'
            '  be(0x1C00 + 640, 0x1780); be(0x1C00 + 48, 0x1C80); be(0x1C80 + 28, 0x1CC0);\n'
            '  pf(0x1CC0 + 16 + 32, 1.0f); pf(0x1CC0 + 16 + 40, 0.0f); pf(0x1C00 + 624, across == 2 ? 1.0f : 0.5f); pf(0x1C00 + 632, 0.0f);\n'
            '  float dt; memcpy(&dt, &t[11], 4);\n'
            '  Context c{}; c.r3.u32 = 0x1000; c.r4.u32 = 0x1200; c.r1.u32 = 0x2F00; c.r13.u32 = 0x1E00; c.f1.f64 = dt;\n'
            '  sub_%s(c, nullptr); float f = float(c.f1.f64); uint32_t fwd; memcpy(&fwd, &f, 4);\n' % fn +
            '  for (int j = 0; j < %d; j++) printf("%%08X ", q[j]);\n' % width +
            '  printf("-> %%08X %%d %%08X", fwd, (int)rd(0x1000 + 420), rd(0x1000 + 440));\n'
            '  for (int e = 0; e < 4; e++) printf(" %%02X", (*pointer(0x1000 + 60 + 12 * e + 8)));\n'
            '  for (int e = 0; e < 4; e++) printf(" %%08X %%08X", rd(0x1000 + 60 + 12 * e), rd(0x1000 + 64 + 12 * e));\n'
            '  printf("' + nl + '");\n'
            '}\n')
    # the game object's pointer F-30960 reads (lis r11 / addi r11 / lwz r3,0(r11) before its +132 test), from its own code
    src = idx[f'sub_{fn}'].read_text(errors='replace')
    src = src[src.index(f'DEFINE_REX_FUNC(sub_{fn})'):]
    asm = [m.group(1).strip() for m in re.finditer(r'^\s*// (.+)$', src[:src.index('\n}')], re.M)]
    k = next(i for i, x in enumerate(asm) if x == 'lwz r3,0(r11)')
    hi = next(int(x.split(',')[1]) for x in reversed(asm[:k]) if x.startswith('lis r11,'))
    lo = next(int(x.split(',')[2]) for x in reversed(asm[:k]) if x.startswith('addi r11,r11,'))
    gptr = ((hi & 0xFFFF) << 16) + lo
    body = body.replace('GAMEPTR', '0x%08Xu' % gptr).replace('DIV', div)
    body = body.replace('%%', '%')  # (the lines with no % operator keep theirs doubled)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n' + body + '}\n')
    stubs = {
        f'sub_{addr(a.tool, "F-20809")}': 'uint32_t w = load(0x1500 + 4 * ((ctx.r3.u32 - (0x1200 + 64)) / 20)); float f; memcpy(&f, &w, 4); ctx.f1.f64 = f;',
        f'sub_{addr(a.tool, "F-50349")}': '',
        f'sub_{addr(a.tool, "F-65049")}': 'ctx.r3.u32 = 0x1C00;',
        f'sub_{addr(a.tool, "F-14900")}': 'throw std::runtime_error("F-14900 reached");',
    }
    exe = build(idx, [f'sub_{fn}'], main_src, a.out, 'controller_quiet' if a.quiet else 'controller_pick', stubs=stubs, vector=True)
    print('# entries x 4 (active reset player a dist margin target), on x 4, band_a min_a max_a band_d min_d max_d bonus speed want waited wait dt across'
          ' -> f1 +420 +440 flags x 4, (+0 +4) x 4 (F-30960)')
    print(run(exe, a.image), end='')


if __name__ == '__main__':
    main()
