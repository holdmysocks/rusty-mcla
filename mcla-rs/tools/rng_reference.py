"""The game's multiply-with-carry random numbers, run offline: the generated code unchanged on flat guest memory.
Each case line of <out-dir>/rng_ref.txt is "<tag> <input words> -> <output words>" (hex words). Sections:

  seed     F-37317 seed(s): in s; out x, c, +8 byte, +12 word (the command-line override F-75950 a stub that leaves s).
  int      F-32828 RAND_INT_RANGE(lo, hi) on the script generator: in x, c, lo, hi; out result, x, c.
  float    F-47737 RAND_FLOAT_RANGE(lo, hi): in x, c, lo, hi (single words); out result (single), x, c.
  diff     F-40328 RAND_INT_RANGE_DIFFERENT(cur, lo, hi): in x, c, cur, lo, hi; out result, x, c.
  ptfx     F-61280 the same draw with the generator passed (r3 generator, r4 cur, r5 lo, r6 hi): as diff.

The script generator is reached as the original reaches it (the running thread F-95075 stubbed to answer the thread,
+48 its context, F-75481 real: +20 the generator). A trap (hi == lo in the DIFFERENT draws: a division by zero the
original traps on) prints "trap" and is not compared.

Run: PYTHONPATH=mcla-rs/tools python mcla-rs/tools/rng_reference.py --recomp D:/MCLA-static
--image <original-image.bin> --out-dir <dir> --tool <mcla-tool.exe> (every harness run under the memcap job of
recomp_closure)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure
from cite_safe import cite_safe

recomp_closure.EXTRA += ('struct Trap{};' + chr(10) + 'static void ppc_trap(Context&,const uint8_t*,int){throw Trap{};}' + chr(10))

NL = chr(92) + 'n'
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def state(rnd):
    r = rnd.random()
    if r < 0.05:
        return [rnd.choice((0, 1, 0xFFFFFFFF, 0x80000000, 0x7FFFFFFF)), rnd.choice((0, 1, 0x10001, 0xFFFFFFFF, 0x5CDCFAA6))]
    return [rnd.getrandbits(32), rnd.getrandbits(32)]


def small(rnd):
    r = rnd.random()
    if r < 0.6:
        return rnd.randint(-5, 20)
    if r < 0.85:
        return rnd.randint(-1000, 100000)
    if r < 0.95:
        return rnd.choice((0x7FFFFFFF, -0x80000000, 0x7FFFFFFE, -0x7FFFFFFF, 1 << 30, -(1 << 30)))
    return rnd.getrandbits(32) - (1 << 31)


def u(v):
    return v & 0xFFFFFFFF


def fl(rnd):
    r = rnd.random()
    if r < 0.5:
        return fb(rnd.uniform(-10, 10))
    if r < 0.8:
        return fb(rnd.uniform(0, 1))
    if r < 0.9:
        return fb(rnd.uniform(-1e6, 1e6))
    return rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x7f7fffff, 0x00000001))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = {k: addr(a.tool, v) for k, v in dict(seed='F-37317', over='F-75950', int='F-32828', float='F-47737', diff='F-40328',
                                              ptfx='F-61280', thread='F-95075').items()}
    rnd = random.Random(61280)
    gen = 'store(0x1000+48,0x1100);store(0x1100+20,0x1200);store(0x1200,c[0]);store(0x1200+4,c[1]);'
    st = 'load(0x1200)'
    secs = []
    cs = [[rnd.choice((0, 1, 0xFFFFFFFF, 0x10000, 0xFFFF)) if rnd.random() < 0.1 else rnd.getrandbits(32)] for _ in range(2000)]
    secs.append(('seed', cs, 'store(0x1200+8,0xAAAAAAAA);store(0x1200+12,0xBBBBBBBB);ctx.r3.u32=0x1200;ctx.r4.u32=c[0];', f['seed'], 4,
                 '(i==0?load(0x1200):i==1?load(0x1204):i==2?uint32_t(*pointer(0x1208)):load(0x120c))'))
    cs = []
    for _ in range(4000):
        lo = small(rnd)
        hi = lo + rnd.randint(-3, 12) if rnd.random() < 0.6 else small(rnd)
        cs.append(state(rnd) + [u(lo), u(hi)])
    secs.append(('int', cs, gen + 'ctx.r3.u32=c[2];ctx.r4.u32=c[3];', f['int'], 3, '(i==0?ctx.r3.u32:i==1?load(0x1200):load(0x1204))'))
    cs = []
    for _ in range(4000):
        lo = fl(rnd)
        hi = fl(rnd) if rnd.random() < 0.7 else lo
        cs.append(state(rnd) + [lo, hi])
    secs.append(('float', cs, gen + 'ctx.f1.f64=double(bf(c[2]));ctx.f2.f64=double(bf(c[3]));', f['float'], 3,
                 '(i==0?fb(float(ctx.f1.f64)):i==1?load(0x1200):load(0x1204))'))
    for tag in ('diff', 'ptfx'):
        cs = []
        for _ in range(4000):
            lo = small(rnd)
            hi = lo + rnd.randint(-3, 12) if rnd.random() < 0.7 else small(rnd)
            cur = rnd.randint(min(lo, hi) - 2, max(lo, hi) + 2) if rnd.random() < 0.8 and abs(hi - lo) < 1 << 20 else small(rnd)
            cs.append(state(rnd) + [u(cur), u(lo), u(hi)])
        setup = (gen + 'ctx.r3.u32=c[2];ctx.r4.u32=c[3];ctx.r5.u32=c[4];' if tag == 'diff' else
                 'store(0x1200,c[0]);store(0x1200+4,c[1]);ctx.r3.u32=0x1200;ctx.r4.u32=c[2];ctx.r5.u32=c[3];ctx.r6.u32=c[4];')
        secs.append((tag, cs, setup, f[tag], 3, '(i==0?ctx.r3.u32:i==1?load(0x1200):load(0x1204))'))

    stubs = {'sub_' + f['thread']: 'ctx.r3.u64=0x1000;', 'sub_' + f['over']: ''}
    body = ''
    for tag, cs, setup, fn, nout, outc in secs:
        w = max(len(c) for c in cs)
        rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c + [0] * (w - len(c))) for c in cs)
        body += ('{static const uint32_t C[][%d]={%s%s%s};' % (w, nl, rows, nl) +
                 'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;' + setup +
                 'try{sub_%s(ctx,nullptr);}catch(Trap&){printf("trap%s");fflush(stdout);continue;}' % (fn, NL) +
                 'for(int i=0;i<%d;i++)printf("%%08x ",uint32_t(%s));printf("%s");fflush(stdout);}}' % (nout, outc, NL) + nl)
    names = {'sub_' + s[3] for s in secs}
    main_src = ('static float bf(uint32_t w){float f;memcpy(&f,&w,4);return f;}' + nl +
                'static uint32_t fb(float f){uint32_t w;memcpy(&w,&f,4);return w;}' + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), sorted(names), main_src, a.out_dir, 'rng', stubs=stubs)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == sum(len(s[1]) for s in secs), (len(out), sum(len(s[1]) for s in secs))
    lines, k = [], 0
    for tag, cs, *_ in secs:
        for c in cs:
            lines.append('%s %s -> %s' % (tag, ' '.join('%08x' % x for x in c), out[k].strip()))
            k += 1
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'rng_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print({s[0]: len(s[1]) for s in secs}, 'traps', sum(1 for x in out if x.strip() == 'trap'))


if __name__ == '__main__':
    main()
