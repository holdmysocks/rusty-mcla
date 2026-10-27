"""The normal-deviate generator (F-66400, Marsaglia's polar method with the spare value kept) run offline: unchanged generated code with the game's natural
logarithm (F-96865) in its closure. Object r3: +0 generator low word, +4 carry, +8 spare flag, +12 spare value; f1 = mean, f2 = deviation.
Sequences of 16 calls from a random generator state (so both the polar branch and the spare branch run), mean / deviation random or 0 / 1.
Line:  NRM <lo> <carry> | <mean> <dev> x16 -> per call: <result> <flag> <spare> <lo> <carry>      (floats as hex bits)
Usage: random_normal_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

NORMAL = 'F-66400'

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);uint32_t lo,carry;const uint32_t O=0x40010000;
 while(in>>std::hex>>lo>>carry){rt_reset();store(O,lo);store(O+4,carry);store(O+8,0);store(O+12,0);
  for(int i=0;i<16;i++){uint32_t m,d;in>>m>>d;float fm,fd;std::memcpy(&fm,&m,4);std::memcpy(&fd,&d,4);
   Context c=fresh();c.r3.u32=O;c.f1.f64=fm;c.f2.f64=fd;sub_NORMAL(c,nullptr);float r=float(c.f1.f64);uint32_t rb;std::memcpy(&rb,&r,4);
   printf("%08x %u %08x %08x %08x\n",rb,load(O+8)>>24,load(O+12),load(O),load(O+4));}
 }
}
'''


def f32bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    fn = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', NORMAL], capture_output=True, text=True, check=True).stdout.split()[1]
    exe = rd.build(idx, [fn], MAIN.replace('sub_NORMAL', fn), a.work, 'random_normal', image=rd.load_image(a.image))
    rnd = random.Random(2033)
    fb = f32bits
    seqs = []
    for _ in range(300):
        lo, carry = rnd.getrandbits(32), rnd.getrandbits(32)
        calls = []
        for _ in range(16):
            m, d = rnd.choice([(0.0, 1.0), (0.0, 0.5), (5.0, 3.0), (rnd.uniform(-10, 10), rnd.uniform(0, 5))])
            calls.append(f'{fb(m):08x} {fb(d):08x}')
        seqs.append(f'{lo:08x} {carry:08x} ' + ' '.join(calls))
    cases = a.work / 'random_normal_cases.txt'
    cases.write_text('\n'.join(seqs) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == 300 * 16
    out = [f'NRM {s.split(" ", 2)[0]} {s.split(" ", 2)[1]} | {s.split(" ", 2)[2]} -> ' + ' | '.join(rows[i * 16:(i + 1) * 16]) for i, s in enumerate(seqs)]
    keep = [l for l in out if not re.search(r'82[0-9a-f]{6}', l)]      # tracked data must not hold raw-address-looking words
    a.out.write_text(chr(10).join(keep) + chr(10))
    print(len(keep), 'of', len(out), 'sequences')


if __name__ == '__main__':
    main()
