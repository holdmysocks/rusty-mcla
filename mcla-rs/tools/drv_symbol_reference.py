"""The car driver machine's expression symbols (mcCarAnimMachine's symbol table, its vtable slot 2, run offline once into a list
"<kind> <name> obj=<ctx|drv> fn=<addr>": `work/drvmachine/car_symtab.txt`) each run offline on the original's code (generated code
unchanged) for random contexts and tunes: r3 the context (whose +16 is the tune), f1 / f2 the case's two numbers for Abs / Min / Max.
The three that read the anim player or the scene (IsCutscenePlaying, ReadTimeRemaining, ReadAnimPhase) are inputs of the
transition tests and are left out.
Usage: drv_symbol_reference.py <out-dir> --symtab <car_symtab.txt> [--recomp D:/MCLA-static] [--image original-image.bin] [--n 1000]
Writes <out-dir>/drv_symbol_ref.txt: a header line with the symbols' names, then per case "ctx words +0..+168 (43) tune words
+0..+148 (38) x y -> one value per symbol" (hex words; a B symbol's r3 byte, the others' f1 as a single)."""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import recomp_dispatch as rd

SKIP = {'IsCutscenePlaying', 'ReadTimeRemaining', 'ReadAnimPhase'}
CTX, TUNE = 0x40810000, 0x40830000


def f2w(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


MAIN = r"""
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); uint32_t w;
 for(;;){
  rt_reset();
  for(int i=0;i<43;i++){ if(!(in>>std::hex>>w)) return 0; store(CTX_+4*i,w);} store(CTX_+16,TUNE_);
  for(int i=0;i<38;i++){ in>>std::hex>>w; store(TUNE_+4*i,w);}
  uint32_t xw,yw; in>>std::hex>>xw>>yw; float x,y; memcpy(&x,&xw,4); memcpy(&y,&yw,4);
  CALLS
  putchar(10); }
}
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--symtab', type=Path, required=True)
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('--n', type=int, default=1000)
    a = ap.parse_args()
    work = a.out.resolve(); work.mkdir(parents=True, exist_ok=True)
    syms, seen = [], set()
    for l in open(a.symtab):
        p = l.split()
        if len(p) < 4 or not p[3].startswith('fn=') or p[1] in SKIP or p[1] in seen:
            continue
        seen.add(p[1]); syms.append((p[0], p[1], 'sub_' + p[3][3:].upper()))
    calls = []
    for kind, _, fn in syms:
        c = 'c.r3.u64=CTX_; c.f1.f64=x; c.f2.f64=y; ' + fn + '(c,nullptr); '
        if kind == 'B':
            calls.append('{Context c=fresh(); ' + c + 'printf(" %02x",c.r3.u32&0xff);}')
        else:
            calls.append('{Context c=fresh(); ' + c + 'float r=(float)c.f1.f64; uint32_t rw; memcpy(&rw,&r,4); printf(" %08x",rw);}')
    main_src = MAIN.replace('CALLS', '\n  '.join(calls)).replace('CTX_', '0x%08Xu' % CTX).replace('TUNE_', '0x%08Xu' % TUNE)
    idx = rd.index(a.recomp); img = rd.load_image(a.image)
    exe = rd.build(idx, sorted({fn for _, _, fn in syms}), main_src, work, 'drv_symbol', image=img)
    rnd = random.Random(20261107)
    near = [0.0, 0.015, -0.015, 0.1, -0.1, 0.25, 1.0, -1.0, 2.5, 15.0, 0.15, 0.5, 25.0, 55.0, 100.0, 11.676, 24.587, 45.204]

    def fl():
        if rnd.random() < 0.5:
            x = rnd.choice(near)
            return x if rnd.random() < 0.6 else x + rnd.choice((-1, 1)) * rnd.choice((1e-6, 1e-3, 0.01))
        return rnd.uniform(-120, 120)
    cases = []
    while len(cases) < a.n:
        ctx = [f2w(fl()) for _ in range(43)]
        ctx[1] = rnd.choice(list(range(22)) + [20] * 4)                     # +4 gesture
        ctx[2] = rnd.getrandbits(32)                                        # +8 flags
        ctx[3] = rnd.getrandbits(32)                                        # +12 flags
        ctx[12] = rnd.getrandbits(32) & 0x0101FFFF                          # +48 / +49 bytes
        ctx[18] = rnd.getrandbits(32) & 0x00010100                          # +73 / +74 bytes
        ctx[31] = rnd.choice((0, 0, 1, 0xFFFFFFFF, 2))                       # +124 shift
        tune = [f2w(fl()) for _ in range(38)]
        tune[1] = rnd.getrandbits(32) & 0x01010101; tune[2] = rnd.getrandbits(32) & 0x01010101   # +4 .. +11 bools
        if rnd.random() < 0.5:                                              # the speed at a band's edge
            k = rnd.randrange(4)
            band = struct.unpack('>f', struct.pack('>I', tune[12 + k]))[0]
            edge = struct.unpack('>f', struct.pack('>f', band * 0.44704))[0] + rnd.choice((0.0, 0.5))
            ctx[24] = f2w(edge) + rnd.choice((-1, 0, 0, 1))
        c = ctx + tune + [f2w(fl()), f2w(fl())]
        if any(w >> 24 == 0x82 for w in c):                                 # (no word that reads as an address of the image)
            continue
        cases.append(c)
    cf = work / 'drv_symbol_cases.txt'
    cf.write_text(''.join(' '.join(f'{w:08x}' for w in c) + '\n' for c in cases))
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    head = ' '.join(f'{k}:{n}' for k, n, _ in syms)
    (work / 'drv_symbol_ref.txt').write_text(head + '\n' + ''.join(' '.join(f'{w:08x}' for w in c) + ' ->' + o + '\n'
                                                                   for c, o in zip(cases, out)))
    print(len(out), 'cases', len(syms), 'symbols')


if __name__ == '__main__':
    main()
