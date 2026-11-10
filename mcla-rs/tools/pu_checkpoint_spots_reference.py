"""PickUpManager_CreatePowerUpSourcesFromCheckPoints F-28343 (and its tail-call wrapper F-80951) run offline (the original's generated
code unchanged, vector mode; the guest sin / cos, the pool allocation through the runtime's allocator, the element constructors).
Stubbed (said so): the level probe F-20855 (a recording stub that keeps its segment and answers "hit") and the element's final type
set-up (it reads the session's options record and rolls the type: not geometry).

The list (r3): +4 the checkpoint array (368 bytes each), +8 the count (u16). A checkpoint: +64 position, +80 the line's other end,
+112 the draw point (set equal to +64 here: the port's `CpIn.pos` stands for both), +168 radius, +176 height, +330 the line flag.
Arguments: r4 the `ordered` flag. Result: a pool of 160-byte elements, in the order they were added.

Line (hex float patterns): "<ordered> <n> {<line> <pos 3> <end 3> <radius> <height>} x n -> <m> {<probe start 3> <probe end 3> <+112> <+20> <+21> <+23>} x m"
Cases with one checkpoint are left out: the original then reads the memory before the array for the last one (the port guards it).
Usage: pu_checkpoint_spots_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import numpy as np
import recomp_dispatch as rd
from cite_safe import cite_safe

MAIN = r'''
uint32_t seg[256];int nseg=0;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFFu);store(0x82860930+4*k,0);} store(0x8286095C,0); store(0x8286093C,0x3F800000u);   // start-up vector constants: xyz mask, (0,0,0,1)
 for(int k=0;k<4;k++){store(0x828609E0+4*k,0x7f800000u);store(0x828608C0+4*k,0);store(0x827D8450+4*k,0x3f800000u);store(0x827D8460+4*k,0x3f000000u);}   // +inf, 0, 1.0, 0.5 vectors
 const uint32_t ARR=0x40101000u;
 uint32_t ordered,n;
 while(std::scanf("%x %u",&ordered,&n)==2){
  rt_reset();std::memset(arena+0x100000,0,0x4000);nseg=0;
  store(0x1000+4,ARR);store(0x1000+8,uint32_t(n)<<16);                          // count: u16 at +8 (the high half of the word)
  for(uint32_t i=0;i<n;i++){uint32_t line,p[3],e[3],rad,h;
   std::scanf("%x %x %x %x %x %x %x %x %x",&line,&p[0],&p[1],&p[2],&e[0],&e[1],&e[2],&rad,&h);
   uint32_t o=ARR+368*i;
   for(int k=0;k<3;k++){store(o+64+4*k,p[k]);store(o+80+4*k,e[k]);store(o+112+4*k,p[k]);}
   store(o+168,rad);store(o+176,h);*pointer(o+330)=uint8_t(line);}
  Context c=fresh();c.r3.u32=0x1000;c.r4.u32=ordered;
  sub_SPOTS(c,nullptr);
  uint32_t P=c.r3.u32;uint32_t m=load16(P+4);uint32_t base=load(P);
  std::printf("-> %u",m);
  for(uint32_t i=0;i<m;i++){uint32_t e=base+160*i;
   for(int k=0;k<6;k++)std::printf(" %x",seg[6*i+k]);
   std::printf(" %x %x %x %x",load(e+112),unsigned(*pointer(e+20)),unsigned(*pointer(e+21)),unsigned(*pointer(e+23)));}
  std::printf("\n");}
}
'''

PROBE = ('extern uint32_t seg[];extern int nseg;if(nseg<40){uint32_t a=ctx.r4.u32;for(int k=0;k<3;k++)seg[6*nseg+k]=load(a+4*k);'
         'for(int k=0;k<3;k++)seg[6*nseg+3+k]=load(a+16+4*k);nseg++;}ctx.r3.u64=1;')


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
    spots, probe = addr('F-28343'), addr('F-20855')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [spots], MAIN.replace('sub_SPOTS', spots), a.work, 'pu_cp_spots', image=img,
                   stubs={probe: PROBE, addr('F-18300'): ''})
    rnd = random.Random(28343)
    cases = []
    radii = [0.0, 3.0, 5.99, 6.0, 8.99, 9.0, 9.01, 10.0, 12.0, 14.99, 15.0, 15.01, 18.0, 20.0, 21.0, 24.0, 27.0, 30.0, 33.0, 40.0, 100.0, -2.0]
    lens = [0.0, 3.0, 4.99, 5.0, 5.01, 8.0, 10.0, 11.99, 12.0, 14.0, 17.9, 20.0, 26.0, 40.0, 80.0]
    for _ in range(1500):
        n = rnd.randint(2, 6)
        cps = []
        for i in range(n):
            line = rnd.random() < 0.4
            p = [f32(rnd.choice([float(rnd.randint(-100, 100)), rnd.uniform(-400, 400)])) for _ in range(3)]
            if line:
                ang = rnd.uniform(0, 6.2831853)
                import math
                ln = rnd.choice(lens + [rnd.uniform(0, 60)])
                e = [f32(p[0] + math.cos(ang) * ln), f32(p[1] + rnd.choice([0.0, 0.0, 3.0, -2.0])), f32(p[2] + math.sin(ang) * ln)]
                if ln == 0.0:
                    ln = 7.0  # (a zero-length line: the original's direction is NaN-free zero as well, but the port's data never has one)
                    e = [f32(p[0] + math.cos(ang) * ln), e[1], f32(p[2] + math.sin(ang) * ln)]
            else:
                e = [0.0, 0.0, 0.0]
            rad = f32(rnd.choice(radii + [rnd.uniform(0, 40)]))
            h = f32(rnd.choice([4.0, 10.0, 50.0, rnd.uniform(1, 100)]))
            cps.append((int(line), p, e, rad, h))
        if False:  # (a coincident next point gives the zero direction in both; not generated)
            cps[1] = (cps[1][0], list(cps[0][1]), cps[1][2], cps[1][3], cps[1][4])
        cases.append((rnd.randint(0, 1), cps))
    inp = ''
    for o, cps in cases:
        inp += '%x %d' % (o, len(cps))
        for line, p, e, rad, h in cps:
            inp += ' %x %s %s %x %x' % (line, ' '.join('%x' % bits(x) for x in p), ' '.join('%x' % bits(x) for x in e), bits(rad), bits(h))
        inp += chr(10)
    p = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    a.out.write_text(cite_safe('\n'.join(i + ' ' + o for i, o in zip(inp.splitlines(), out)) + '\n'))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
