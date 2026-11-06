"""The HUD map's tick run offline (recomp_dispatch): F-98009 (map, dt) - the clock +468 += dt, the checkpoint strobe +460 by
its speed +476 (fused), back to 0 past (checkpoints + 1) x the period +480, the frame count +452 + 1 and the pulse +464 by its
speed +532 (fused) wrapped into [0, pi] by F-43389. The map's +24 is 0, so the checkpoint count comes from F-88065 on +388
(a stub that reads the count from the word +388 points at). Writes ui_map_tick_ref.txt, one case per line, floats as bits:
  t468 s460 p464 f452 speed476 period480 pspeed532 dt n -> t468 s460 p464 f452
Usage: ui_map_tick_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import math
import os
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t v[9];
 while(scanf("%u %u %u %u %u %u %u %u %u",&v[0],&v[1],&v[2],&v[3],&v[4],&v[5],&v[6],&v[7],&v[8])==9){
  rt_reset();
  uint32_t m=rt_alloc(1024,16),cnt=rt_alloc(16,16);
  for(uint32_t i=0;i<1024;i+=4)store(m+i,0);
  store(cnt,v[8]);store(m+388,cnt);store(m+24,0);
  store(m+468,v[0]);store(m+460,v[1]);store(m+464,v[2]);store(m+452,v[3]);store(m+476,v[4]);store(m+480,v[5]);store(m+532,v[6]);
  Context c=fresh();c.r3.u32=m;float f;memcpy(&f,&v[7],4);c.f1.f64=f;
  sub_TICK(c,nullptr);
  printf("OUT %u %u %u %u\n",load(m+468),load(m+460),load(m+464),load(m+452));fflush(stdout);
 }
}
'''


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 5000
    r = random.Random(98009)
    PI = f32(math.pi)
    cases = []
    for i in range(count):
        dt = f32(r.choice([1 / 30, 1 / 60, 1 / 144, 0.0, r.uniform(0, 0.25), r.uniform(0, 2.0)]))
        ps = f32(r.choice([4.0, r.uniform(0, 12)]))
        ss = f32(r.choice([4.6, r.uniform(0, 10)]))
        per = f32(r.choice([1.2, r.uniform(0.1, 3)]))
        n = r.choice([0, 1, 3, 8, r.randrange(0, 40)])
        k = r.random()
        if k < 0.3:     # the pulse lands on pi (or one ulp either side)
            p = f32(PI - f32(ps * dt))
            p = struct.unpack('>f', struct.pack('>I', max(0, fbits(p) + r.choice([-1, 0, 0, 1]))))[0] if p > 0 else p
        else:
            p = f32(r.uniform(0, PI))
        lim = f32(f32(n + 1.0) * per)
        if k > 0.7:     # the strobe lands on its bound (or one ulp either side)
            s = f32(lim - f32(ss * dt))
            s = struct.unpack('>f', struct.pack('>I', max(0, fbits(s) + r.choice([-1, 0, 1]))))[0] if s > 0 else s
        else:
            s = f32(r.uniform(0, lim * 1.2))
        t = f32(r.uniform(0, 5000))
        cases.append([fbits(t), fbits(s), fbits(p), r.choice([r.randrange(0, 1 << 20), 0xFFFFFFFF]), fbits(ss), fbits(per), fbits(ps), fbits(dt), n])
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_maptick'
    work.mkdir(exist_ok=True)
    tick, count_fn = 'sub_' + addr('F-98009'), 'sub_' + addr('F-88065')
    exe = rd.build(idx, [tick], MAIN.replace('sub_TICK', tick), work, 'maptick', image=img,
                   stubs={count_fn: 'ctx.r3.u64=load(ctx.r3.u32);'})
    inp = ''.join('%u %u %u %u %u %u %u %u %u\n' % tuple(c) for c in cases)
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[4:] for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%u %u %u %u %u %u %u %u %u -> %s\n' % (*c, o) for c, o in zip(cases, res)))
    wraps = sum(int(o.split()[2]) == fbits(PI) for o in res)
    print(len(res), 'cases,', wraps, 'pulses left on pi')


if __name__ == '__main__':
    main()
