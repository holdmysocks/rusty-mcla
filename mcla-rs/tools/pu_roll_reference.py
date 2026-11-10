"""A power-up source's type roll run offline: the original's own code F-14458 (object +22 the RANDOM flag, +128 the allowed-type
bits, +136 the type out, +144 the Mystery flag in / out; r4 the random number) with its callee F-52455 (the n-th allowed type; reads
the byte popcount table in the image), unchanged generated code, nothing stubbed.

Cases: the allowed bits (every subset of the low 12 bits, random 32-bit words, empty, all ones), the number r (0 .. 40, powers of
two, random 31-bit, 0x7fffffff, negative words), the RANDOM flag 0 / 1 and the Mystery flag the source had 0 / 1.
Line: "<ok> <bits> <r> <was> -> <+136 type> <+144 mystery>" (hex words; r as the 32-bit word).
Usage: pu_roll_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from cite_safe import cite_safe

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t ok,bits,r,was;
 while(std::scanf("%x %x %x %x",&ok,&bits,&r,&was)==4){
  rt_reset();
  *pointer(0x1000+22)=uint8_t(ok);store(0x1000+128,bits);store(0x1000+136,0xdeadbeefu);*pointer(0x1000+144)=uint8_t(was);
  Context c=fresh();c.r3.u32=0x1000;c.r4.u32=r;
  sub_ROLL(c,nullptr);
  std::printf("%x %x %x %x -> %x %x\n",ok,bits,r,was,load(0x1000+136),unsigned(*pointer(0x1000+144)));}
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    roll = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', 'F-14458'], capture_output=True, text=True, check=True).stdout.split()[1]
    idx = rd.index(a.recomp)
    exe = rd.build(idx, [roll], MAIN.replace('sub_ROLL', roll), a.work, 'pu_roll')
    rnd = random.Random(14458)
    cases = []
    rs = list(range(0, 41)) + [1 << k for k in range(31)] + [0x7FFFFFFF, 0xFFFFFFFF, 0x80000000, 0xFFFFFFFC, 0xFFFFFF00]
    for bits in range(0, 4096):
        for _ in range(2):
            cases.append((rnd.randint(0, 1), bits, rnd.choice(rs) if rnd.random() < 0.5 else rnd.getrandbits(31), rnd.randint(0, 1)))
    for _ in range(3000):
        bits = rnd.choice([rnd.getrandbits(32), rnd.getrandbits(32) & rnd.getrandbits(32), 0xFFFFFFFF, 0, 1 << rnd.randrange(32), rnd.getrandbits(26)])
        cases.append((rnd.randint(0, 1), bits, rnd.choice(rs) if rnd.random() < 0.5 else rnd.getrandbits(31), rnd.randint(0, 1)))
    for bits in (0, 1, 0x3FF, 0xFFFFFFFF, 0x80000000, 0x301):
        for r in rs:
            for ok in (0, 1):
                for was in (0, 1):
                    cases.append((ok, bits, r, was))
    inp = ''.join('%x %x %x %x\n' % c for c in cases)
    p = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    a.out.write_text(cite_safe('\n'.join(out) + '\n'))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
