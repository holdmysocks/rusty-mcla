"""The sky's key tracks run offline as generated (cameras2-map4), written to crates/game/src/testdata/map_sky_ref.txt for the
port's tests. recomp_closure imports tools/memcap.py, so every process of this script (g++, the harness) runs in a 4 GB job.
The key-slope derivation F-18740 is run on the key records first (48-byte records: time +0, value +16, slope +32,
count u16 at +28 of the track), then the sampler F-72455 and the sun-direction sampler F-19682 as generated. Nothing is stubbed.
One line per case: "SKY <t> <n> <time v0 v1 v2 v3>... -> <sample x y z w> <normalised x y z w>"; floats are f32 bits in hex.
Usage: map_sky_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t K=0x40010000,KEYS=0x40010400,OUT=0x40010100,OUT2=0x40010200;
 while(in>>k){rt_reset();
  int n;uint32_t tb;in>>std::hex>>tb>>std::dec>>n;
  for(uint32_t a=K;a<K+0x100;a++)*pointer(a)=0;
  for(uint32_t a=KEYS;a<KEYS+0x800;a++)*pointer(a)=0xA5;
  for(int i=0;i<4;i++)store(INF_+4*i,0x7F800000u);     // the +inf splat the rsqrt guard compares with
  store(K+24,KEYS);store16(K+28,n);
  for(int i=0;i<n;i++){uint32_t w[5];in>>std::hex>>w[0]>>w[1]>>w[2]>>w[3]>>w[4];store(KEYS+48*i,w[0]);for(int c=0;c<4;c++)store(KEYS+48*i+16+4*c,w[1+c]);}
  float t;
  {uint32_t b=tb;std::memcpy(&t,&b,4);}
  {Context cx=fresh();cx.r3.u32=K;sub_SLOPE(cx,nullptr);}
  for(int i=0;i<4;i++)store(OUT+4*i,0);
  uint32_t sw;
  {Context cx=fresh();cx.r3.u32=OUT2;cx.r5.u32=K;cx.f1.f64=double(t);sub_SAMPLE(cx,nullptr);float w=float(cx.f1.f64);std::memcpy(&sw,&w,4);}
  printf("%08x %08x %08x %08x",load(OUT2),load(OUT2+4),load(OUT2+8),sw);
  for(int i=0;i<4;i++)store(OUT+4*i,0);
  {Context cx=fresh();cx.r3.u32=OUT;cx.r4.u32=K-4;cx.f1.f64=double(t);sub_SUN(cx,nullptr);}
  printf(" %08x %08x %08x %08x\n",load(OUT),load(OUT+4),load(OUT+8),load(OUT+12));
 }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--seed', type=int, default=19682)
    ap.add_argument('--n', type=int, default=12000)
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    a.work.mkdir(parents=True, exist_ok=True)

    def name(fid):
        return 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    rnd = random.Random(a.seed)
    inf_splat = ((-32122 << 16) & 0xFFFFFFFF) + 2528     # the +inf vector the sun sampler's reciprocal-root guard compares with
    src = MAIN.replace('sub_SLOPE', name('F-18740')).replace('sub_SAMPLE', name('F-72455')).replace('sub_SUN', name('F-19682')).replace('INF_', '0x%08Xu' % inf_splat)
    exe = rd.build(idx, [name('F-18740'), name('F-72455'), name('F-19682')], src, a.work, 'map_sky', image=img)
    cases = []
    for c in range(a.n):
        n = rnd.choice([1, 2, 3, 4, 5, 7, 8, 9, 12, 2, 7, 7])
        base = rnd.choice([0.0, 0.0, 0.0, 3.0, -2.0])
        ts = sorted(f32(base + rnd.choice([rnd.uniform(0, 24), rnd.uniform(0, 24), float(rnd.randrange(0, 25)), rnd.uniform(0, 1)])) for _ in range(n))
        if rnd.random() < 0.15 and n > 2:
            ts[rnd.randrange(1, n)] = ts[rnd.randrange(0, n - 1)]       # a repeated time (zero-length segment)
            ts.sort()
        if rnd.random() < 0.05 and n > 1:
            ts = sorted(ts, reverse=True)                               # not increasing (the loader derives zero slopes)
        keys = []
        for i in range(n):
            kind = rnd.random()
            if kind < 0.08:
                v = [0.0, 0.0, 0.0, rnd.uniform(-1, 1)]
            elif kind < 0.12:
                v = [rnd.uniform(-1e-9, 1e-9) for _ in range(3)] + [0.0]
            elif kind < 0.20:
                v = [rnd.choice([0.0, 1.0, -1.0]) for _ in range(3)] + [rnd.choice([0.0, 1.0])]
            elif kind < 0.25:
                v = [rnd.uniform(-1e3, 1e3) for _ in range(4)]
            else:
                v = [rnd.uniform(-1, 1) for _ in range(4)]
            keys.append([ts[i]] + [f32(x) for x in v])
        if rnd.random() < 0.4:
            t = rnd.choice(ts)
        elif rnd.random() < 0.2:
            t = rnd.choice([0.0, 24.0, ts[0], ts[-1], ts[0] - 0.5, ts[-1] + 0.5, -1.0, 30.0])
        elif rnd.random() < 0.3:
            t = f32(rnd.choice(ts) + rnd.choice([1e-6, -1e-6, 1e-3, -1e-3]))
        else:
            t = rnd.uniform(0, 24)
        t = f32(t)
        cases.append(f'SKY {fb(t):08x} {n} ' + ' '.join(' '.join(f'{fb(x):08x}' for x in k) for k in keys))
    cf = a.work / 'SKY_cases.txt'
    cf.write_text('\n'.join(cases) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    outs = p.stdout.splitlines()
    assert len(outs) == len(cases), (len(outs), len(cases))
    a.out.write_text('\n'.join(f'{c} -> {o}' for c, o in zip(cases, outs)) + '\n')
    print(len(cases), 'cases')


if __name__ == '__main__':
    main()
