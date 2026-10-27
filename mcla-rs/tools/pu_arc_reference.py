"""The lightning arc's per-frame clock step (F-49462) run offline: unchanged generated code, the game-wide generator in guest memory.
Arc object r3: +0 colour, +16 / +24 per-arc draws, +20 growth rate, +28 flicker rate, +32 texture offset, +40 seed (u64), +48 growth clock,
+52 flicker clock, +56 -> the style (+0 ColorMin, +16 ColorMax, +48 ArcGrowth, +56 GrowthSpeed, +64 FluxTangent, +80 FlickerSpeed, +92 TexAnimRate).
Cases: random clocks (past 1 in about half), rates, dt (0, small, large), style ranges, generator state.
Line: AT <lo> <carry> <dt> <grow> <flick> <grate> <frate> <tex> <cmin x4> <cmax x4> <ag x2> <gs x2> <ft x2> <fs x2> <texrate> -> <colour x4> <+16> <+20> <+24> <+28> <+32> <seed hi> <seed lo> <+48> <+52> <lo> <carry>
Usage: pu_arc_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

ARC_TICK = 'F-49462'
ARC, STYLE = 0x1000, 0x1200

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);uint32_t lo,carry,dtb;
 const uint32_t RNG=(0x8286u<<16)+0x880;
 while(in>>std::hex>>lo>>carry>>dtb){rt_reset();
  store(RNG,lo);store(RNG+4,carry);
  uint32_t w[22];for(int i=0;i<22;i++)in>>w[i];
  // grow flick grate frate tex cmin4 cmax4 ag2 gs2 ft2 fs2 texrate
  store(ARC+48,w[0]);store(ARC+52,w[1]);store(ARC+20,w[2]);store(ARC+28,w[3]);store(ARC+32,w[4]);store(ARC+56,STYLE_);
  for(int i=0;i<4;i++){store(STYLE_+4*i,w[5+i]);store(STYLE_+16+4*i,w[9+i]);}
  store(STYLE_+48,w[13]);store(STYLE_+52,w[14]);store(STYLE_+56,w[15]);store(STYLE_+60,w[16]);store(STYLE_+64,w[17]);store(STYLE_+68,w[18]);
  store(STYLE_+80,w[19]);store(STYLE_+84,w[20]);store(STYLE_+92,w[21]);
  Context c=fresh();c.r3.u32=ARC;float dt;std::memcpy(&dt,&dtb,4);c.f1.f64=dt;sub_TICK(c,nullptr);
  printf("R");for(int i=0;i<4;i++)printf(" %08x",load(ARC+4*i));
  for(int o:{16,20,24,28,32,40,44,48,52})printf(" %08x",load(ARC+o));
  printf(" %08x %08x\n",load(RNG),load(RNG+4));}
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
    addr = subprocess.run([a.tool, 'registry', 'addr', ARC_TICK], capture_output=True, text=True, check=True).stdout.split()[1]
    fn = 'sub_' + addr
    src = MAIN.replace('STYLE_', hex(STYLE)).replace('ARC', hex(ARC)).replace('sub_TICK', fn)
    src = src.replace('0x1000(', 'ARC(')
    exe = rd.build(idx, [fn], src, a.work, 'pu_arc', image=rd.load_image(a.image))
    if True:
        seed = 2028
        rnd = random.Random(seed)
        fb = f32bits
        lines = []
        for _ in range(1620):
            lo, carry = rnd.getrandbits(32), rnd.getrandbits(32)
            dt = rnd.choice([0.0, rnd.uniform(0, 0.03), rnd.uniform(0, 0.03), rnd.uniform(0.03, 0.4)])
            r = [rnd.uniform(0, 1.3), rnd.uniform(0, 1.3), rnd.uniform(0.3, 30), rnd.uniform(0.3, 30), rnd.uniform(-3, 3)]
            cmin = [rnd.random() for _ in range(4)]
            cmax = [rnd.random() for _ in range(4)]
            def rg(lo_, hi_):
                x, y = rnd.uniform(lo_, hi_), rnd.uniform(lo_, hi_)
                return [min(x, y), max(x, y)]
            rngs = rg(0, 2) + rg(0.05, 2) + rg(0, 3) + rg(0.05, 2) + [rnd.uniform(-2, 2)]
            words = [fb(v) for v in r + cmin + cmax + rngs]
            lines.append(f'{lo:08x} {carry:08x} {fb(dt):08x} ' + ' '.join('%08x' % w for w in words))
        cases = a.work / 'pu_arc_cases.txt'
        cases.write_text('\n'.join(lines) + '\n')
        p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
        if p.returncode:
            raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
        res = [f'AT {cl} -> {ol.split(" ", 1)[1]}' for cl, ol in zip(lines, p.stdout.splitlines())]
        assert len(res) == len(lines)
        keep = [l for l in res if not re.search(r'82[0-9a-f]{6}', l)][:1500]      # tracked data must not hold raw-address-looking words
        assert len(keep) == 1500
        res = keep
        a.out.write_text(chr(10).join(keep) + chr(10))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
