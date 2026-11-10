"""A laser layer's scale parameters F-92307 run offline (the original's generated code unchanged, vector mode, with its callee the 1D
gradient noise F-52219 and the floor helper it calls; the permutation and gradient tables are in the image).
The record (r3): seven floats +0 .. +24 (x scale, offset, sway, sway rate, brightness, flicker rate, flicker depth); f1 the time; the
four-float result is written at r4: (x scale, 0, sway noise term, flicker term).

Line (hex float patterns): "<7 record floats> <t> -> <4 result floats>". Cases: the real layer records of the port (LASER_LAYERS, passed in
by the test data), random records, t from 0 to 200 s and negative, noise arguments on integers and half integers.
Usage: pu_laser_params_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
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
 uint32_t w[8];
 while(true){ bool ok=true;for(int k=0;k<8;k++)ok=ok&&std::scanf("%x",&w[k])==1; if(!ok)break;
  rt_reset();
  for(int k=0;k<7;k++)store(0x1000+4*k,w[k]);
  for(int k=0;k<4;k++)store(0x1100+4*k,0xdead0000u+k);
  Context c=fresh();c.r3.u32=0x1000;c.r4.u32=0x1100;{float f;std::memcpy(&f,&w[7],4);c.f1.f64=f;}
  sub_LASER(c,nullptr);
  std::printf("->");for(int k=0;k<4;k++)std::printf(" %x",load(0x1100+4*k));std::printf("\n");}
}
'''


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return float(np.float32(x))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    laser = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', 'F-92307'], capture_output=True, text=True, check=True).stdout.split()[1]
    idx = rd.index(a.recomp)
    exe = rd.build(idx, [laser], MAIN.replace('sub_LASER', laser), a.work, 'pu_laser', image=None)
    rnd = random.Random(92307)
    cases = []
    for _ in range(4000):
        rec = [f32(rnd.choice([0.0, 1.0, rnd.uniform(-3, 3), rnd.uniform(0, 1)])) for _ in range(7)]
        rec[3] = f32(rnd.choice([rnd.uniform(0, 8), 1.0, 0.5, 2.0, 0.0, 4.5]))
        rec[5] = f32(rnd.choice([rnd.uniform(0, 8), 1.0, 0.5, 2.0, 0.0, 4.5]))
        t = f32(rnd.choice([0.0, rnd.uniform(0, 5), rnd.uniform(0, 200), -rnd.uniform(0, 5), float(rnd.randint(0, 30)), rnd.randint(0, 60) / 2.0, 1 / 60, 1e-7]))
        cases.append(tuple(bits(x) for x in rec) + (bits(t),))
    inp = ''.join(' '.join('%x' % x for x in c) + '\n' for c in cases)
    p = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    a.out.write_text(cite_safe('\n'.join(' '.join('%x' % x for x in c) + ' ' + o for c, o in zip(cases, out)) + '\n'))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
