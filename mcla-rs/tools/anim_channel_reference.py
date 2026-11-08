"""Run the original's animation float-channel evaluators offline on random channels.

Three leaf functions of the recomp, each on its own channel object in a scratch area:
  F-89511  quantised channel, the value of frame i (i clamped to count - 1): fma(q, scale, offset)
  F-74386  quantised channel, frames i and i + 1 lerped by fr: fma(b - a, fr, a) on the two values
  F-18785  raw float channel, keys i and i + 1 lerped by fr
Writes one line per case: the inputs and the result's bits, all floats as hex words, for the test
anim_channels_match_the_original (crates/formats/src/rage/anim.rs).
"""
import argparse
import random
import struct
from pathlib import Path
from recomp_leaf import compile_and_run, extract, shim
import memcap

memcap.apply()

FUNCS = ['sub_825460A0', 'sub_82546120', 'sub_82541688']

MAIN = r'''
static uint64_t load64(uint32_t a) { return uint64_t(load(a)) << 32 | load(a + 4); }
static void store64(uint32_t a, uint64_t v) { store(a, uint32_t(v >> 32)); store(a + 4, uint32_t(v)); }
#define REX_LOAD_U64(a) load64(a)
#define REX_STORE_U64(a,v) store64(a,v)
FUNCS
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    std::ifstream bin(argv[1], std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin), {});
    if (image.size() != 0x9e0000) return 3;
    std::ifstream cases(argv[2]);
    int kind; uint32_t nb, count, sc, off, i, fr, n;
    while (cases >> kind >> std::hex >> nb >> count >> sc >> off >> i >> fr >> n) {
        std::memset(objects, 0, sizeof(objects));
        std::vector<uint32_t> w(n);
        for (auto &x : w) cases >> x;
        cases >> std::dec;
        for (uint32_t k = 0; k < n; k++) store(0x1100 + 4 * k, w[k]);
        Context ctx{};
        ctx.r1.u32 = 0x2f00; ctx.r3.u32 = 0x1000; ctx.r4.u32 = i; ctx.r6.u32 = 0x2f80;
        store(0x1008, 0x1100);
        float frf; std::memcpy(&frf, &fr, 4); ctx.f1.f64 = frf;
        uint32_t out = 0;
        if (kind == 2) {
            sub_82541688(ctx, nullptr); out = load(0x2f80);
        } else {
            store(0x100c, nb); store(0x1010, count); store(0x1014, sc); store(0x1018, off);
            if (kind == 0) { sub_825460A0(ctx, nullptr); float v = float(ctx.f1.f64); std::memcpy(&out, &v, 4); }
            else { sub_82546120(ctx, nullptr); out = load(0x2f80); }
        }
        std::printf("%d %x %x %09x %09x %x %09x %x", kind, nb, count, sc, off, i, fr, n);
        for (auto x : w) std::printf(" %09x", x);
        std::printf(" -> %09x\n", out);
    }
}
'''


def f2u(v):
    return struct.unpack('>I', struct.pack('>f', v))[0]


def rnd_float(rng):
    return rng.choice([
        lambda: rng.uniform(-1, 1),
        lambda: rng.uniform(-5000, 5000),
        lambda: rng.uniform(-1e-3, 1e-3),
        lambda: struct.unpack('>f', struct.pack('>I', rng.getrandbits(32) & 0xbfffffff))[0],
    ])()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    for a in ['recomp', 'image', 'out']:
        ap.add_argument('--' + a, type=Path, required=True)
    ap.add_argument('--compiler', default='g++')
    ap.add_argument('--cases', type=int, default=6000)
    args = ap.parse_args()
    gen = args.recomp / 'generated/default'
    funcs = ''
    for f in FUNCS:
        for p in sorted(gen.glob('mcla_recomp.*.cpp')):
            t = p.read_text()
            if f'DEFINE_REX_FUNC({f}) {{' in t:
                funcs += extract(p, f)
                break
        else:
            raise SystemExit(f'{f} not found')
    rng = random.Random(82546120)
    lines = []
    for c in range(args.cases):
        kind = c % 3
        count = rng.randint(2, 48)
        nb = rng.randint(1, 32)
        if kind == 2:
            vals = [rnd_float(rng) for _ in range(count)]
            if rng.random() < 0.3:  # neighbouring keys close together, as in clips
                base = rnd_float(rng)
                vals = [base + rng.uniform(-1e-2, 1e-2) for _ in range(count)]
            words = [f2u(v) for v in vals]
            nb, sc, off = 0, 0, 0
            i = rng.randint(0, count - 2)
        else:
            words = [rng.getrandbits(32) for _ in range((count + 1) * nb // 32 + 3)]
            if rng.random() < 0.5:  # a clip's own scale: the range over 2^nb - 1
                lo = rng.uniform(-10, 10)
                sc, off = f2u(rng.uniform(1e-4, 20) / ((1 << nb) - 1)), f2u(lo)
            else:
                sc, off = f2u(rnd_float(rng)), f2u(rnd_float(rng))
            i = rng.randint(0, count + 3) if kind == 0 else rng.randint(0, count - 2)
        fr = rng.choice([0.0, 1.0, 0.5, rng.random(), rng.random(), rng.random()])
        lines.append(f'{kind} {nb:x} {count:x} {sc:09x} {off:09x} {i:x} {f2u(fr):09x} {len(words):x} '
                     + ' '.join(f'{w:09x}' for w in words))
    src = shim().replace('#include <vector>', '#include <vector>\n#include <ios>') + MAIN.replace('FUNCS', funcs)
    out = compile_and_run(args.compiler, src, '\n'.join(lines) + '\n', args.image, args.out, 'anim_channel')
    print(f'ran the three evaluators on {len(lines)} cases -> {out}')


if __name__ == '__main__':
    main()
