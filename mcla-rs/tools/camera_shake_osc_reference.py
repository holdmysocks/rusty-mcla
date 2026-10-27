"""The camera shake oscillator (F-17992) run offline: unchanged generated code with its closure (the game's sine, F-10707's helper) and the game-wide
random generator in guest memory. Arguments: r3 out vector, r4 phase vector (in / out, advanced by frequency x (f1 x f3)), r5 frequency vector, r6 amplitude
vector, r8 random vector, f1 and f3 (the caller passes 1.0 and the frame time), f2 the block's multiplier m.
out[j] = (amp[j] sin(phase[j]) + (-rnd[j] + 2 rnd[j] u_j)) m with u_j the generator's 23-bit numbers, x then y then z.
Line:  OSC <lo> <carry> <dt> <m> <phase x3> <freq x3> <amp x3> <rnd x3> -> <out x3> <phase x3> <lo> <carry>      (floats as hex bits)
Usage: camera_shake_osc_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

OSC = 'F-17992'

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);uint32_t lo,carry,dt,m;
 const uint32_t RNG=(0x8286u<<16)+0x880;const uint32_t OUT=0x40010000,PH=0x40010100,FR=0x40010200,AM=0x40010300,RD=0x40010400;
 while(in>>std::hex>>lo>>carry>>dt>>m){rt_reset();store(RNG,lo);store(RNG+4,carry);
  for(uint32_t a:{PH,FR,AM,RD}){for(int i=0;i<3;i++){uint32_t w;in>>w;store(a+4*i,w);}store(a+12,0);}
  for(int i=0;i<4;i++)store(OUT+4*i,0);
  float fdt,fm,one=1.0f;std::memcpy(&fdt,&dt,4);std::memcpy(&fm,&m,4);
  Context c=fresh();c.r3.u32=OUT;c.r4.u32=PH;c.r5.u32=FR;c.r6.u32=AM;c.r8.u32=RD;c.f1.f64=one;c.f2.f64=fm;c.f3.f64=fdt;sub_OSC(c,nullptr);
  printf("R");for(int i=0;i<3;i++)printf(" %08x",load(OUT+4*i));for(int i=0;i<3;i++)printf(" %08x",load(PH+4*i));printf(" %08x %08x\n",load(RNG),load(RNG+4));}
}
'''


def f32bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    fn = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', OSC], capture_output=True, text=True, check=True).stdout.split()[1]
    exe = rd.build(idx, [fn], MAIN.replace('sub_OSC', fn), a.work, 'camera_shake_osc', image=rd.load_image(a.image))
    if True:
        seed = 2032
        rnd = random.Random(seed)
        fb = f32bits
        lines = []
        for _ in range(2160):
            dt = rnd.choice([1 / 60, 1 / 30, rnd.uniform(0.001, 0.2)])
            m = rnd.choice([0.0, rnd.uniform(0, 3), rnd.uniform(0, 30)])
            scale = rnd.choice([1.0, 10.0, 1000.0, 1e5])
            ph = [rnd.uniform(-scale, scale) for _ in range(3)]
            fr = [rnd.choice([50.0, rnd.uniform(0, 120), rnd.uniform(-40, 40)]) for _ in range(3)]
            am = [rnd.choice([0.0, rnd.uniform(-1, 1), rnd.uniform(-0.1, 0.1)]) for _ in range(3)]
            rd_ = [rnd.choice([0.0, rnd.uniform(0, 1), rnd.uniform(0, 0.05)]) for _ in range(3)]
            words = [fb(v) for v in ph + fr + am + rd_]
            lines.append(f'{rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x} {fb(dt):08x} {fb(m):08x} ' + ' '.join('%08x' % w for w in words))
        cases = a.work / 'camera_shake_osc_cases.txt'
        cases.write_text('\n'.join(lines) + '\n')
        p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
        if p.returncode:
            raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
        rows = p.stdout.splitlines()
        assert len(rows) == len(lines)
        keep = [l for l in [f'OSC {cl} -> {r[2:]}' for cl, r in zip(lines, rows)] if not re.search(r'82[0-9a-f]{6}', l)][:2000]      # tracked data must not hold raw-address-looking words
        assert len(keep) == 2000
        res = keep
        a.out.write_text(chr(10).join(keep) + chr(10))
    print(len(rows), 'cases')


if __name__ == '__main__':
    main()
