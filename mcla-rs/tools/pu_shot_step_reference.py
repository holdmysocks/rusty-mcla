"""A shot's per-frame step F-66683 and its first search F-37047 run offline (the original's generated code unchanged, vector mode).
Stubbed (said so, both recording): the target search F-33277 (verified separately by pu_targets_reference.py; here it answers the
case's list and logs its arguments: the mega flag r7, the range f1, the width f2) and the landing F-82026 (logs the target entry
and the shot's kind).

The shot object (r3): +4 the type, +28 ShootRange, +32 width, +37 the found flag; the racer (r4): +68 the holder whose +44.. is the list
of found targets, +104 its count, +108 the clock since; the frame time is the clock object's +8 at 0x827D7508.

Lines (hex; floats as bit patterns):
  S <kind> <found> <wait> <dt> <range> <width> <n listed> <ids> <n the search finds> <ids> -> <found'> <wait'> <searches> <mega> <range> <width> <count'> <lands: id kind ...>
  F <kind> <old wait> <range> <width> <n the search finds> <ids> -> <found'> <wait'> <searches> <mega> <range> <width> <count'>      (F-37047, the first search)
Usage: pu_shot_step_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import numpy as np
import recomp_dispatch as rd
from cite_safe import cite_safe

MAIN = r'''
uint32_t lst[8];int nlst=0,searches=0,rec_mega=0,rec_range=0,rec_width=0,nland=0;uint32_t lands[32];
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t S=0x1000,R=0x1100,H=0x1200;
 char kind;
 while(std::scanf(" %c",&kind)==1){
  uint32_t k,found=0,wait,dt=0,range,width,nl=0,ids[8],ns=0;
  rt_reset();searches=0;nland=0;nlst=0;
  if(kind=='S'){std::scanf("%x %x %x %x %x %x %u",&k,&found,&wait,&dt,&range,&width,&nl);for(uint32_t i=0;i<nl;i++)std::scanf("%x",&ids[i]);}
  else{std::scanf("%x %x %x %x",&k,&wait,&range,&width);}
  std::scanf("%u",&ns);for(uint32_t i=0;i<ns;i++)std::scanf("%x",&lst[i]);nlst=ns;
  store(R+68,H);store(S+4,k);store(S+28,range);store(S+32,width);*pointer(S+37)=uint8_t(found);
  store(H+108,wait);store(0x827D7508,dt);
  if(kind=='S'){for(uint32_t i=0;i<nl;i++)store(H+44+4*i,ids[i]);store(H+104,nl);}
  else{store(H+104,0xdeadu);}
  Context c=fresh();c.r3.u32=S;c.r4.u32=R;
  if(kind=='S')sub_STEP(c,nullptr);else sub_FIRST(c,nullptr);
  std::printf("-> %u %x %d %d %x %x %u",unsigned(*pointer(S+37)),load(H+108),searches,rec_mega,rec_range,rec_width,load(H+104));
  if(kind=='S'){for(int i=0;i<nland;i++)std::printf(" %x %u",lands[2*i],lands[2*i+1]);}
  std::printf("\n");}
}
'''

STUB_SEARCH = ('extern uint32_t lst[];extern int nlst,searches,rec_mega,rec_range,rec_width;searches++;rec_mega=ctx.r7.u32&0xff;'
               '{float f=float(ctx.f1.f64);memcpy(&rec_range,&f,4);f=float(ctx.f2.f64);memcpy(&rec_width,&f,4);}'
               'store(ctx.r6.u32+60,0);for(int i=0;i<nlst;i++)store(ctx.r6.u32+4*i,lst[i]);store(ctx.r6.u32+60,nlst);ctx.r3.u64=nlst?1:0;')
STUB_LAND = ('extern int nland;extern uint32_t lands[];lands[2*nland]=ctx.r5.u32;lands[2*nland+1]=ctx.r6.u32;nland++;')


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
    addr = lambda f: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[1]
    stepf, first, search = addr('F-66683'), addr('F-37047'), addr('F-33277')
    land = addr('F-82026')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [stepf, first], MAIN.replace('sub_STEP', stepf).replace('sub_FIRST', first), a.work, 'pu_shot_step', image=img,
                   stubs={search: STUB_SEARCH, land: STUB_LAND})
    rnd = random.Random(66683)
    lines = []
    for _ in range(3000):
        kind = rnd.choice([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 25])
        found = rnd.randint(0, 1)
        dt = f32(rnd.choice([1 / 60, 1 / 60, 1 / 30, 0.0, 0.1, rnd.uniform(0, 0.3), 0.25]))
        wait = f32(rnd.choice([0.0, 0.25, step(0.25, 1), step(0.25, -1), f32(0.25 - dt), step(f32(0.25 - dt), 1), step(f32(0.25 - dt), -1),
                               rnd.uniform(0, 0.5), 1e-3, 5.0, 0.2]))
        listed = [0x3000 + 0x40 * rnd.randint(1, 9) for _ in range(rnd.randint(0, 5))] if found else [0x3000 + 0x40 * rnd.randint(1, 9) for _ in range(rnd.randint(0, 3))]
        finds = [0x4000 + 0x40 * rnd.randint(1, 9) for _ in range(rnd.choice([0, 0, 1, 1, 2, 5]))]
        lines.append('S %x %x %x %x %x %x %d %s %d %s' % (kind, found, bits(wait), bits(dt), bits(f32(rnd.choice([20.0, 40.0, 100.0]))), bits(f32(rnd.choice([10.0, -1.0, 5.0]))),
                                                          len(listed), ' '.join('%x' % x for x in listed), len(finds), ' '.join('%x' % x for x in finds)))
    for _ in range(600):
        kind = rnd.choice([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 25])
        finds = [0x4000 + 0x40 * rnd.randint(1, 9) for _ in range(rnd.choice([0, 0, 1, 1, 2, 5]))]
        lines.append('F %x %x %x %x %d %s' % (kind, bits(f32(rnd.choice([0.0, 0.125, 3.0, 1e-5]))), bits(f32(rnd.choice([20.0, 40.0, 100.0]))), bits(f32(rnd.choice([10.0, -1.0, 5.0]))),
                                           len(finds), ' '.join('%x' % x for x in finds)))
    inp = '\n'.join(lines) + '\n'
    p = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(lines), (len(out), len(lines))
    a.out.write_text(cite_safe('\n'.join(f'{c} {o}' for c, o in zip(lines, out)) + '\n'))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
