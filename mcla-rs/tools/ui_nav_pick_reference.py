"""The GPS map's pick at the stick's release run offline (recomp_dispatch): F-97746 (the career target manager's vtable slot 139)
with its force flag, as F-63282 calls it with the cursor NM+448; the element list *(0x829061A4)+12 walked by F-46639's iterator as it
is. Writes ui_nav_pick_ref.txt, one case per line, numbers as float bits in hex:
  K radius cx cz n (x y z b8 b9)*n -> r index      the elements in the iterator's order; r = the answer, index = the element handed to
                                                   the manager's slot 141 (-1 none)
Usage: ui_nav_pick_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
POS = 0x7FFF0010   # the element's vtable slot 3: its position (a fake address)
SET = 0x7FFF0014   # the manager's vtable slot 141: the element picked
GLOBALS = 0x7FFF0020  # never called: carries the global below into the source


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 char kind[4];
 while(scanf("%3s",kind)==1){
  rt_reset();PICKED=-1;
  uint32_t rad,cx,cz,n;scanf("%x %x %x %x",&rad,&cx,&cz,&n);
  uint32_t nm=rt_alloc(2048,16),h=rt_alloc(64,16),bk=rt_alloc(64,16),evt=rt_alloc(64,16),mvt=rt_alloc(1024,16),mg=rt_alloc(256,16),cur=rt_alloc(16,16);
  for(uint32_t i=0;i<2048;i+=4)store(nm+i,0);
  store(0x8286D814u,nm);store(nm+568,rad);store(0x829061A4u,h);
  store(h+12,bk);store(h+16,0x00030000u);for(int b=0;b<3;b++)store(bk+4*b,0);
  store(evt+12,POS_);store(mvt+564,SET_);store(mg,mvt);
  uint32_t tail[3]={0,0,0};
  for(uint32_t k=0;k<n;k++){
   uint32_t x,y,z,b8,b9;scanf("%x %x %x %x %x",&x,&y,&z,&b8,&b9);
   uint32_t e=rt_alloc(64,16),nd=rt_alloc(16,16);
   store(e,evt);store(e+4,k);store(e+16,x);store(e+20,y);store(e+24,z);store(e+28,0);
   *pointer(e+8)=uint8_t(b8);*pointer(e+9)=uint8_t(b9);
   store(nd+4,e);store(nd+8,0);
   int b=k%3;if(tail[b])store(tail[b]+8,nd);else store(bk+4*b,nd);tail[b]=nd;
  }
  store(cur,cx);store(cur+4,0);store(cur+8,cz);store(cur+12,0);
  Context c=fresh();c.r3.u32=mg;c.r4.u32=cur;c.r5.u32=1;sub_PICK(c,nullptr);
  printf("OUT %d %d\n",int(c.r3.u32&0xFF),PICKED);fflush(stdout);
 }
}
'''


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 3000
    r = random.Random(97746)
    cases = []
    for _ in range(count):
        rad = r.choice([90.0, 500.0, r.uniform(90, 500)])
        cx, cz = r.uniform(-3000, 3000), r.uniform(-2400, 2200)
        n = r.randrange(0, 12)
        els = []
        for _ in range(n):
            k = r.random()
            if k < 0.15:
                x, z = cx, cz                                    # on the cursor (distance 0)
            elif k < 0.3 and els:
                x, z = els[r.randrange(len(els))][0], els[r.randrange(len(els))][2]
            else:
                s = r.choice([20, 100, 400, 2000])
                x, z = cx + r.uniform(-s, s), cz + r.uniform(-s, s)
            b8 = 1 if r.random() < 0.1 else 0
            b9 = 0 if r.random() < 0.1 else 1
            els.append((x, r.uniform(-20, 60), z, b8, b9))
        # (the iterator's order: bucket k % 3, each in insertion order)
        order = [e for b in range(3) for i, e in enumerate(els) if i % 3 == b]
        cases.append('K %x %x %x %x' % (fbits(rad), fbits(cx), fbits(cz), n) + ''.join(' %x %x %x %x %x' % (fbits(x), fbits(y), fbits(z), b8, b9) for x, y, z, b8, b9 in els) + ' |' + ''.join(' %d' % els.index(e) for e in order))
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_navpick'
    work.mkdir(exist_ok=True)
    pick = 'sub_' + addr('F-97746')
    main_src = MAIN.replace('POS_', '0x%Xu' % POS).replace('SET_', '0x%Xu' % SET).replace('sub_PICK', pick)
    exe = rd.build(idx, [pick], main_src, work, 'navpick', image=img,
                   fakes={GLOBALS: '}static int PICKED;static void unused_g_(Context &ctx,const uint8_t *base){', POS: 'for(int i=0;i<16;i+=4)store(ctx.r4.u32+i,load(ctx.r3.u32+16+i));',
                          SET: 'PICKED=int(load(ctx.r4.u32+4));'})
    inp = '\n'.join(c.split(' |')[0] for c in cases) + '\n'
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[3:].strip('\n') for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%s ->%s\n' % (c, o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
