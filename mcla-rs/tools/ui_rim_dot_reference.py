"""One minimap rim dot run offline (recomp_dispatch): F-42900, the rim dot draw, on
an element {angle +0, colour +4, kind +12, selected byte +16} with the UI clock word set; its mesh draw F-74141 is a stub that
records what it is handed: the mesh (the table slot: kind 2 -> +4, kind 10 -> +8, else +0), the two matrices at r5 (32
floats), the alpha f1 and f2, r4 / r6 / r7 and the byte r1+87. Writes ui_rim_dot_ref.txt, floats as bits:
  angle kind selected clock -> slot alpha f2 r4 r6 r7 b87 m0..m31
Usage: ui_rim_dot_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import math
import os
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
CLOCK = 0x827D7514  # the UI clock the selected dot's pulse reads (+20 of the block at 0x827D7500)


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def fid(a):
    return subprocess.run([TOOL, 'registry', 'id', a], capture_output=True, text=True, check=True).stdout.split()[0]


MAIN = r'''
static uint32_t TAB;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t v[4];
 while(scanf("%u %u %u %u",&v[0],&v[1],&v[2],&v[3])==4){
  rt_reset();
  uint32_t me=rt_alloc(64,16),e=rt_alloc(64,16);TAB=me;
  for(uint32_t i=0;i<64;i+=4){store(me+i,0);store(e+i,0);}
  store(me+0,0xA000u);store(me+4,0xA004u);store(me+8,0xA008u);
  store(e+0,v[0]);store(e+4,0x11223344u);store(e+12,v[1]);
  store(CLOCK_,v[3]);
  Context c=fresh();c.r3.u32=me;c.r4.u32=e;
  image[e+16-IMAGE_BASE_]=0; // (placeholder, replaced below)
  sub_DOT(c,nullptr);
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 3000
    r = random.Random(82240)
    f32 = lambda x: struct.unpack('>f', struct.pack('>f', x))[0]
    fb = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
    cases = []
    for i in range(count):
        a = f32(r.choice([r.uniform(-math.pi, math.pi), r.uniform(-20, 20), r.choice([0.0, math.pi / 2, math.pi, -math.pi / 2, 3 * math.pi / 4])]))
        kind = r.choice([0, 1, 2, 10, 3, 7])
        sel = r.random() < 0.5
        clock = f32(r.choice([r.uniform(0, 30), r.uniform(0, 4000), 0.0]))
        cases.append([fb(a), kind, int(sel), fb(clock)])
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_rimdot'
    work.mkdir(exist_ok=True)
    dot, draw = 'sub_' + addr('F-42900'), 'sub_' + addr('F-74141')
    main_src = MAIN.replace('sub_DOT', dot).replace('CLOCK_', '0x%Xu' % CLOCK)
    # (the selected byte: a plain store of the guest byte)
    main_src = main_src.replace("  image[e+16-IMAGE_BASE_]=0; // (placeholder, replaced below)\n", "")
    main_src = main_src.replace("store(e+12,v[1]);", "store(e+12,v[1]);store(e+16,v[2]<<24);")
    rec = ('{uint32_t s=ctx.r3.u32==0xA000u?0:ctx.r3.u32==0xA004u?4:ctx.r3.u32==0xA008u?8:99;float a=float(ctx.f1.f64),b=float(ctx.f2.f64);'
           'uint32_t ab,bb;memcpy(&ab,&a,4);memcpy(&bb,&b,4);printf("OUT %u %u %u %u %u %u %u",s,ab,bb,ctx.r4.u32,ctx.r6.u32,ctx.r7.u32,'
           '(unsigned)(load(ctx.r1.u32+84)&255));for(int i=0;i<128;i+=4)printf(" %u",load(ctx.r5.u32+i));putchar(10);fflush(stdout);}')
    exe = rd.build(idx, [dot], main_src, work, 'rimdot', image=img, stubs={draw: rec})
    inp = ''.join('%u %u %u %u\n' % tuple(c) for c in cases)
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[4:] for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%u %u %u %u -> %s\n' % (*c, o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
