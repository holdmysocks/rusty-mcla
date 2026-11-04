"""The original's button draw (F-28198) run offline (recomp_dispatch): unchanged generated code on a flat guest memory.
A button with random records is drawn under a random parent matrix / colour transform in state 1, 2 or 3; each record's character is
a fake object whose draw slot prints the draw state it is handed. Writes ui_button_ref.txt, one case per line, numbers in hex:
  BT  state  parent m[6] cx[8]  n  (flags m[6] cx[8]) x n  ->  (record m[6] cx[8] alpha state) per drawn record
Floats are f32 bit patterns; cx = 4 multipliers then 4 additions (signed bytes).
Usage: ui_button_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

FAKE_DRAW = 0x7F000020
TOOL = __import__('os').environ.get('MCLA_TOOL', 'mcla-tool')
RELEASE = ('75034', '99877', '75050')


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
MAIN = r'''
static uint32_t fb(float v){uint32_t b;std::memcpy(&b,&v,4);return b;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t state;
 while(scanf("%x",&state)==1){
  rt_reset();
  uint32_t par=rt_alloc(160,16),btn=rt_alloc(32,16),child=rt_alloc(16,16),vt=rt_alloc(32,16);
  for(uint32_t i=0;i<160;i+=4)store(par+i,0);
  uint32_t v;for(int i=0;i<6;i++){scanf("%x",&v);store(par+24+4*i,v);}
  for(int i=0;i<8;i++){scanf("%x",&v);*pointer(par+56+i)=uint8_t(v);}
  uint32_t n;scanf("%x",&n);uint32_t arr=rt_alloc(40*n+8,16);
  store(child,vt);store(vt+12,0x7F000020u);
  for(uint32_t k=0;k<n;k++){uint32_t r=arr+40*k;scanf("%x",&v);*pointer(r)=uint8_t(v);*pointer(r+1)=0;*pointer(r+2)=0;*pointer(r+3)=0;
   for(int i=0;i<6;i++){scanf("%x",&v);store(r+4+4*i,v);}
   for(int i=0;i<8;i++){scanf("%x",&v);*pointer(r+28+i)=uint8_t(v);}
   store(r+36,child);}
  for(uint32_t i=0;i<32;i+=4)store(btn+i,0);store(btn+12,arr);*pointer(btn+21)=uint8_t(n);
  printf("OUT");
  Context c=fresh();c.r3.u32=btn;c.r4.u32=par;c.r5.u32=state;sub_DRAW(c,nullptr);
  printf("\n");fflush(stdout);
 }
}
'''
PRINT = ('uint32_t s=ctx.r4.u32;for(int i=0;i<6;i++)printf(" %x",load(s+24+4*i));for(int i=0;i<8;i++)printf(" %x",*pointer(s+56+i));'
         'printf(" %x %x",*pointer(s+71),ctx.r5.u32);')


def f32(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def case(r):
    state = r.choice([1, 2, 3])
    pm = [f32(r.uniform(-3, 3)) for _ in range(4)] + [f32(r.uniform(-900, 900)) for _ in range(2)]
    pcx = [r.randrange(256) for _ in range(8)]
    n = r.randrange(1, 5)
    recs = []
    for _ in range(n):
        recs += [r.randrange(16)] + [f32(r.uniform(-3, 3)) for _ in range(4)] + [f32(r.uniform(-500, 500)) for _ in range(2)] + [r.randrange(256) for _ in range(8)]
    return [state] + pm + pcx + [n] + recs


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 400
    r = random.Random(28198)
    cases = [case(r) for _ in range(count)]
    # identity records (as on the disc) under a few parents
    for k in range(8):
        c = case(r)
        c[15] = 2
        c[16:] = [15] + [f32(1), 0, 0, f32(1), 0, 0] + [64, 64, 64, 64, 0, 0, 0, 0] + [3] + [f32(1), 0, 0, f32(1), 0, 0] + [64, 64, 64, 64, 0, 0, 0, 0]
        cases.append(c)
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_button'
    work.mkdir(exist_ok=True)
    # the draw state's release helpers: their pointers stay null in the local state (+64, +140), so they never run
    stubs = {'sub_' + addr(f): 'throw std::runtime_error("release helper reached");' for f in ('F-%s' % x for x in RELEASE)}
    draw = 'sub_' + addr('F-28198')
    exe = rd.build(idx, [draw], MAIN.replace('sub_DRAW', draw), work, 'button', image=img, fakes={FAKE_DRAW: PRINT}, stubs=stubs)
    inp = '\n'.join(' '.join('%x' % v for v in c) for c in cases) + '\n'
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True, check=True)
    res = [l[3:].strip() for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(cases), (len(res), p.stderr[-500:])
    Path(out).write_text(''.join('BT %s -> %s\n' % (' '.join('%x' % v for v in c), o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
