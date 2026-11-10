"""PickUpManager_AddPowerUpSource F-28622 run offline with the ring F-32756 and the row F-10244 it dispatches to, and the pool's
next-element helper F-59872 (the original's generated code unchanged, vector mode; the guest sin / cos, the vector helpers and the
constructors of the new element run as the original). Stubbed (said so): the level probe F-20855 (a recording stub that keeps its
segment and answers "hit") and the element's final type set-up (it reads the session's options record and rolls the type: not geometry).

The list (r3): +0 the pool's base, +4 the element count (u16); elements are 160 bytes. Arguments: r4 the position (16-byte vector),
f1 the width, r6 the second position vector. Line (hex float patterns; the lane w of both vectors is random - the code masks it):
  <pos 4> <width> <second 4> -> <n> { <start xyz> <end xyz> <+112 as float> <bytes +20 +21 +23> } x n
where start / end are the two ends of the probe segment of each element in the order the elements were added.
Usage: pu_hand_spots_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import numpy as np
import recomp_dispatch as rd
from cite_safe import cite_safe

MAIN = r'''
uint32_t seg[64];int nseg=0;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFFu);store(0x82860930+4*k,0);} store(0x8286095C,0); store(0x8286093C,0x3F800000u);   // start-up vector constants: xyz mask, (0,0,0,1)
 for(int k=0;k<4;k++){store(0x828609E0+4*k,0x7f800000u);store(0x828608C0+4*k,0);store(0x827D8450+4*k,0x3f800000u);store(0x827D8460+4*k,0x3f000000u);}   // +inf, 0, 1.0, 0.5 vectors
 uint32_t w[5];
 while(true){bool ok=true;for(int k=0;k<5;k++)ok=ok&&std::scanf("%x",&w[k])==1;if(!ok)break;
  uint32_t sec[4];for(int k=0;k<4;k++)ok=ok&&std::scanf("%x",&sec[k])==1;
  rt_reset();std::memset(arena+0x100000,0,0x10000);nseg=0;
  store(0x1000,0x40100000u);store(0x1004,0);                  // the pool: base, count (u16 at +4)
  for(int k=0;k<4;k++){store(0x1100+4*k,w[k]);store(0x1110+4*k,sec[k]);}
  Context c=fresh();c.r3.u32=0x1000;c.r4.u32=0x1100;c.r6.u32=0x1110;{float f;std::memcpy(&f,&w[4],4);c.f1.f64=f;}
  SPOTS(c,nullptr);
  uint32_t n=load16(0x1004);
  std::printf("-> %u",n);
  for(uint32_t i=0;i<n;i++){uint32_t e=0x40100000u+160*i;
   for(int k=0;k<6;k++)std::printf(" %x",seg[6*i+k]);
   std::printf(" %x %x %x %x",load(e+112),unsigned(*pointer(e+20)),unsigned(*pointer(e+21)),unsigned(*pointer(e+23)));}
  std::printf("\n");}
}
'''

PROBE = ('extern uint32_t seg[];extern int nseg;if(nseg<10){uint32_t a=ctx.r4.u32;for(int k=0;k<3;k++)seg[6*nseg+k]=load(a+4*k);for(int k=0;k<3;k++)seg[6*nseg+3+k]=load(a+16+4*k);nseg++;}'
         'ctx.r3.u64=1;')


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
    addr = lambda f: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[1]
    spots, probe = addr('F-28622'), addr('F-20855')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [spots], MAIN.replace('SPOTS', spots), a.work, 'pu_hand_spots', image=img, stubs={probe: PROBE, addr('F-18300'): ''})
    rnd = random.Random(28622)
    cases = []
    widths = [0.0, 1.0, 2.9999998, 3.0, 3.0000002, 6.0, 6.4999995, 6.5, 6.5000005, 9.999999, 10.0, 10.000001, 12.999999, 13.0, 13.000001,
              14.999999, 15.0, 15.000001, 25.0, -1.0, -5.0, 1e-30, 1e9]
    for _ in range(3000):
        pos = [f32(rnd.choice([0.0, float(rnd.randint(-50, 50)), rnd.uniform(-500, 500), rnd.uniform(-5, 5)])) for _ in range(3)] + [f32(rnd.uniform(-9, 9))]
        w = f32(rnd.choice(widths + [rnd.uniform(0, 30), rnd.uniform(0, 30)]))
        mode = rnd.choice(['ring', 'ring', 'row', 'row', 'row', 'rowint', 'same', 'negzero', 'tiny', 'huge'])
        if mode == 'ring':
            sec = [0.0, 0.0, 0.0, f32(rnd.uniform(-9, 9))]
        elif mode == 'negzero':
            sec = [-0.0, rnd.choice([0.0, -0.0]), -0.0, 1.0]
        elif mode == 'same':
            sec = pos[:3] + [0.0]
        elif mode == 'tiny':
            sec = [f32(pos[0] + rnd.choice([1e-30, 1e-20, 1e-10, 1e-3])), pos[1], f32(pos[2] + rnd.choice([0.0, 1e-20]))] + [0.0]
        elif mode == 'huge':
            sec = [f32(pos[i] + rnd.choice([1e18, -3e19, 1e30])) for i in range(3)] + [0.0]
        elif mode == 'rowint':
            sec = [f32(pos[0] + rnd.choice([0, 3, -4, 10, 100, -100])), f32(pos[1] + rnd.choice([0, 0, 2.0, -3.0])), f32(pos[2] + rnd.choice([0, 4, -3, 10, 100, -100]))] + [f32(rnd.uniform(-9, 9))]
        else:
            sec = [f32(pos[i] + rnd.uniform(-300, 300)) for i in range(3)] + [f32(rnd.uniform(-9, 9))]
        cases.append(([bits(x) for x in pos], bits(w), [bits(x) for x in sec]))
    inp = ''.join(' '.join('%x' % x for x in c[0] + [c[1]] + c[2]) + chr(10) for c in cases)
    p = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    a.out.write_text(cite_safe('\n'.join(' '.join('%x' % x for x in c[0] + [c[1]]) + ' ' + ' '.join('%x' % x for x in c[2]) + ' ' + o for c, o in zip(cases, out)) + '\n'))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
