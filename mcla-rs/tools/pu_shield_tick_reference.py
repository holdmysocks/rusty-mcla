"""The Shield's per-frame update F-32841 run offline (the original's generated code unchanged, nothing stubbed). The object: +368 and
+372 as the Shield's constructor F-78725 stores them (0.18, 0.24 - read from the image constants the constructor loads), +376 the flash
clock, +380 = 1.0 (the constructor), +384 / +388 / +392 / +396 the timers; the frame time is the clock object's +8 at 0x827D7508
(zero in the static image: set per case); the random generator is the game's multiply-with-carry pair (the low word, then the carry word).

Line (hex words; floats as bit patterns): "<dt> <+376> <+384> <+388> <lo> <carry> -> <+376> <+384> <+388> <+392> <+396> <lo> <carry>".
Cases: dt 1/60 and others (0, tiny, 0.1, random, NaN excluded), flash 0 .. 1.5, clocks around the 0.24 threshold (its ulp neighbours),
random generator states incl. 0 and 0xFFFFFFFF.
Usage: pu_shield_tick_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import numpy as np
import recomp_dispatch as rd
from cite_safe import cite_safe

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t dt,f0,t2,cl,lo,ca;
 while(std::scanf("%x %x %x %x %x %x",&dt,&f0,&t2,&cl,&lo,&ca)==6){
  rt_reset();
  store(0x827D7508,dt);store(0x82860880,lo);store(0x82860884,ca);
  store(0x1000+368,0x3e3851ecu);store(0x1000+372,0x3e75c28fu);store(0x1000+380,0x3f800000u);
  store(0x1000+376,f0);store(0x1000+384,t2);store(0x1000+388,cl);store(0x1000+392,0xdead0001u);store(0x1000+396,0xdead0002u);
  Context c=fresh();c.r3.u32=0x1000;
  sub_TICK(c,nullptr);
  std::printf("-> %x %x %x %x %x %x %x\n",load(0x1000+376),load(0x1000+384),load(0x1000+388),load(0x1000+392),load(0x1000+396),load(0x82860880),load(0x82860884));}
}
'''


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return float(np.float32(x))


def step(x, n):
    y = np.float32(x)
    for _ in range(abs(n)):
        y = np.nextafter(y, np.float32(np.inf if n > 0 else -np.inf))
    return float(y)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    tick = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', 'F-32841'], capture_output=True, text=True, check=True).stdout.split()[1]
    idx = rd.index(a.recomp)
    exe = rd.build(idx, [tick], MAIN.replace('sub_TICK', tick), a.work, 'pu_shield_tick')
    rnd = random.Random(32841)
    cases = []
    th = f32(0.24)
    for _ in range(4000):
        dt = f32(rnd.choice([1 / 60, 1 / 60, 1 / 60, 1 / 30, 0.0, 1e-30, 0.1, rnd.uniform(0, 0.1), rnd.uniform(0, 1 / 30), 0.24, 0.0166667]))
        f0 = f32(rnd.choice([0.0, 1.0, 0.5, rnd.uniform(0, 1), rnd.uniform(0, 1.5), 0.99999, step(1.0, -1), step(1.0, 1), -0.5]))
        t2 = f32(rnd.choice([0.0, 0.18, rnd.uniform(0, 5), 1e-3, 100.0, rnd.uniform(0, 1)]))
        cl = f32(rnd.choice([0.0, th, step(th, 1), step(th, -1), rnd.uniform(0, 0.3), rnd.uniform(0.2, 0.25), f32(th - dt), step(f32(th - dt), 1), step(f32(th - dt), -1)]))
        lo = rnd.choice([0, 1, 0xFFFFFFFF, rnd.getrandbits(32), rnd.getrandbits(32), rnd.getrandbits(32)])
        ca = rnd.choice([0, 1, 0x5CDCFAA6, 0xFFFFFFFF, rnd.getrandbits(32), rnd.getrandbits(32)])
        cases.append((bits(dt), bits(f0), bits(t2), bits(cl), lo, ca))
    inp = ''.join('%x %x %x %x %x %x\n' % c for c in cases)
    p = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    a.out.write_text(cite_safe('\n'.join('%x %x %x %x %x %x %s' % (*c, o) for c, o in zip(cases, out)) + '\n'))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
