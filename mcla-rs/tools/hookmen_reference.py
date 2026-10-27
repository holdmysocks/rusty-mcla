"""Run the original's hook-icon helpers offline and print their results next to the inputs.

Three functions of the recompiled executable are run UNCHANGED (no stubs) on a flat guest memory:
  nearest3 (F-52355): the three nearest candidates to the camera, 80-byte entries at manager +352;
  the hook-icon queue comparator (F-13438): float at record +100 behind two pointers, far first;
  the beat-me-there sort comparator (F-33494): float at +8, ascending;
  the icon queue fill (F-30496): the +100 distance of the three records it queues.
Output (committed as crates/game/tests/hookmen_reference.txt, read by the tests in hookicons.rs and mc4.rs):
  N <camx> <camy> <camz> <n> (<kind> <x> <y> <z> <id>)*n = <idx0> <id0> <idx1> <id1> <idx2> <id2>   (hex f32 / u32 bit patterns)
  Q <a> <b> = <result>        far-first comparator (a, b are f32 bits)
  B <a> <b> = <result>        beat-me-there comparator
  R <flag> (<x> <y> <z>)*3 <camx> <camy> <camz> = <d0> <d1> <d2>   the queue's distance key (F-30496): frame translation to camera, f32 bits
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def bits(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]


def fl(b):
    return struct.unpack('>f', struct.pack('>I', b))[0]


def cases(rng):
    nearest = []
    grid = [0.0, 10.0, 20.0, -10.0, 100.5]
    for i in range(500):
        n = rng.randint(0, 8)
        cam = [rng.uniform(-900, 900), rng.uniform(0, 40), rng.uniform(-900, 900)]
        if i % 5 == 0:  # many exact ties: positions on a coarse grid, camera on the grid too
            cam = [rng.choice(grid), rng.choice(grid), rng.choice(grid)]
            ents = [(rng.choice([0, 1, 6, 9, 14, 16]), rng.choice(grid), rng.choice(grid), rng.choice(grid), rng.randint(0, 3000)) for _ in range(n)]
        elif i % 7 == 0:  # far beyond the 100000 start distance
            ents = [(rng.choice([0, 9, 14]), rng.uniform(-3e5, 3e5), 0.0, rng.uniform(-3e5, 3e5), rng.randint(0, 3000)) for _ in range(n)]
        else:
            ents = [(rng.choice([0, 0, 1, 6, 9, 10, 11, 13, 14, 16]), rng.uniform(-900, 900), rng.uniform(0, 40), rng.uniform(-900, 900), rng.randint(0, 3000)) for _ in range(n)]
        nearest.append((cam, ents))
    special = [0.0, -0.0, 1.0, -1.0, 1.5, 3.0e38, float('inf'), float('-inf'), float('nan'), 1e-30]
    pairs = [(a, b) for a in special for b in special]
    for _ in range(1500):
        a = rng.uniform(-100, 100)
        pairs.append((a, a if rng.random() < .3 else (fl(bits(a) + rng.choice([-1, 1])) if rng.random() < .3 else rng.uniform(-100, 100))))
    return nearest, pairs


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--dest', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nearest_fn, q_fn, b_fn, fill_fn = (addr(a.tool, f) for f in ('F-52355', 'F-13438', 'F-33494', 'F-30496'))
    nearest, pairs = cases(random.Random(20261027))
    rng = random.Random(7)
    fills = [(rng.randint(0, 1), [[rng.uniform(-900, 900), rng.uniform(0, 60), rng.uniform(-900, 900)] for _ in range(3)], [rng.uniform(-900, 900), rng.uniform(0, 60), rng.uniform(-900, 900)]) for _ in range(300)]
    lines = []
    for cam, ents in nearest:
        lines.append('N %08x %08x %08x %d %s' % (bits(cam[0]), bits(cam[1]), bits(cam[2]), len(ents),
                     ' '.join('%x %08x %08x %08x %x' % (k, bits(x), bits(y), bits(z), i) for k, x, y, z, i in ents)))
    for p, q in pairs:
        lines.append('P %08x %08x' % (bits(p), bits(q)))
    for fg, ts, cm in fills:
        lines.append('R %d %s %s' % (fg, ' '.join('%08x %08x %08x' % tuple(bits(v) for v in t) for t in ts), ' '.join('%08x' % bits(v) for v in cm)))
    (a.out).mkdir(parents=True, exist_ok=True)
    inp = a.out / 'cases.txt'
    inp.write_text('\n'.join(lines) + '\n')
    src = r'''
#include <sstream>
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in("%s");std::string ln;
 store(0x8288B9F0,0);store(0x8287E064,0x2600);   // camera table: index 0 at 0x2600, position at +240
 while(std::getline(in,ln)){std::istringstream s(ln);std::string t;s>>t;std::memset(objects,0,sizeof(objects));
  if(t=="N"){uint32_t c[3];unsigned n;s>>std::hex>>c[0]>>c[1]>>c[2]>>std::dec>>n;
   store(0x2600+240,c[0]);store(0x2600+244,c[1]);store(0x2600+248,c[2]);store(0x1000+5472,n);
   for(unsigned i=0;i<n;i++){uint32_t k,x,y,z,id;s>>std::hex>>k>>x>>y>>z>>id;uint32_t e=0x1000+352+80*i;
    store(e,k);store(e+16,x);store(e+20,y);store(e+24,z);store(e+64,id);}
   Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1600;ctx.r5.u32=3;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);
   std::printf("N %%s =",ln.substr(2).c_str());for(int i=0;i<6;i++)std::printf(" %%x",load(0x1600+4*i));std::puts("");}
  else if(t=="R"){unsigned fg;uint32_t v[12];s>>std::dec>>fg>>std::hex;for(int i=0;i<12;i++)s>>v[i];
   store(0x8286D804,0x1000);store(0x827E5640,0);store(0x8288B9F0,2);store(0x8287E064,0x6000);
   store(0x6000+240,v[9]);store(0x6000+244,v[10]);store(0x6000+248,v[11]);
   for(int i=0;i<3;i++)for(int idx:{0,2}){uint32_t n=0x1000+idx*1040+(10+i)*64+7520+48;store(n,v[3*i]);store(n+4,v[3*i+1]);store(n+8,v[3*i+2]);}
   for(int i=0;i<3;i++)store(0x1000+340+4*i,0x1800);
   store(0x5000,0x5100);*pointer(0x5100+72)=uint8_t(fg);
   Context c3{};c3.r3.u32=0x1000;c3.r13.u32=0x5000;c3.r1.u32=0x7000;sub_%s(c3,nullptr);
   std::printf("R %%s =",ln.substr(2).c_str());for(int i=0;i<3;i++)std::printf(" %%x",load(0x1000+5504+112*i+100));std::puts("");}
  else{uint32_t a,b;s>>std::hex>>a>>b;
   store(0x1200+100,a);store(0x1300+100,b);store(0x1100,0x1200);store(0x1104,0x1300);
   Context c1{};c1.r3.u32=0x1100;c1.r4.u32=0x1104;c1.r1.u32=0x2800;sub_%s(c1,nullptr);
   store(0x1400+8,a);store(0x1500+8,b);
   Context c2{};c2.r3.u32=0x1400;c2.r4.u32=0x1500;c2.r1.u32=0x2800;sub_%s(c2,nullptr);
   std::printf("P %%08x %%08x = %%d %%d\n",a,b,(int)c1.r3.s32,(int)c2.r3.s32);}}}
''' % (inp.as_posix(), nearest_fn[4:], fill_fn[4:], q_fn[4:], b_fn[4:])
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + nearest_fn[4:], 'sub_' + q_fn[4:], 'sub_' + b_fn[4:], 'sub_' + fill_fn[4:]], src, a.out, 'hookmen_ref', scratch_end=0x8000)
    a.dest.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
