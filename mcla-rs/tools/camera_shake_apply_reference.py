"""The camera shake's apply (F-10707) run offline: unchanged generated code with its closure (the oscillator F-17992, the game's sine and cosine, the
look-at F-97951, the turns F-52786 / F-83846 / F-53646, EulerXYZ F-28753) and the game-wide random generator in guest memory. Arguments: r3 camFX (the five
blocks at +96 + 224 i), r4 the camera's matrix (rows a, b, c, d), r5 the car's place, f1 the frame time. Each block: CarRot, Rot, Tran (amplitude,
frequency, random, phase vectors), then +192 timer, +196 / +200 the two factors of m, +204 / +208 the Rot / Tran frequency factors (1, as every trigger
writes), +212 state (0 off, 1 one-shot, 2 continuous).
Line:  SA <lo> <carry> <dt> <car x3> <M 12: a b c d, xyz> 5 x (<state> <timer> <f196> <f200> <36 part words: CarRot, Rot, Tran; amp freq rnd phase>)
        -> <M 12> 5 x (<timer> <state> <9 phases>) <lo> <carry>                                                  (floats as hex bits)
Usage: camera_shake_apply_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

APPLY = 'F-10707'

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);uint32_t lo,carry,dt;
 const uint32_t RNG=(0x8286u<<16)+0x880;const uint32_t M=0x40010000,CAR=0x40010100,FX=0x40010200;
 while(in>>std::hex>>lo>>carry>>dt){rt_reset();store(RNG,lo);store(RNG+4,carry);
  // the vector constants the game's start-up fills (zero in the static image): the xyz mask, (0, 0, 0, 1), +inf
  for(int k=0;k<4;k++){store(0x82860950u+4*k,k<3?0xffffffffu:0);store(0x82860930u+4*k,k<3?0:0x3f800000u);store(0x828609E0u+4*k,0x7F800000u);}
  for(int i=0;i<3;i++){uint32_t w;in>>w;store(CAR+4*i,w);}store(CAR+12,0x3f800000u);
  for(int r=0;r<4;r++){for(int i=0;i<3;i++){uint32_t w;in>>w;store(M+16*r+4*i,w);}store(M+16*r+12,r==3?0x3f800000u:0);}
  for(int b=0;b<5;b++){uint32_t B=FX+96+224*b,st,tm,f1,f2;in>>st>>tm>>f1>>f2;
   store(B+192,tm);store(B+196,f1);store(B+200,f2);store(B+204,0x3f800000u);store(B+208,0x3f800000u);store(B+212,st);
   for(int v=0;v<12;v++){for(int i=0;i<3;i++){uint32_t w;in>>w;store(B+16*v+4*i,w);}store(B+16*v+12,0);}}
  float fdt;std::memcpy(&fdt,&dt,4);
  Context c=fresh();c.r3.u32=FX;c.r4.u32=M;c.r5.u32=CAR;c.f1.f64=fdt;sub_APPLY(c,nullptr);
  printf("R");for(int r=0;r<4;r++)for(int i=0;i<3;i++)printf(" %08x",load(M+16*r+4*i));
  for(int b=0;b<5;b++){uint32_t B=FX+96+224*b;printf(" %08x %08x",load(B+192),load(B+212));
   for(int p=0;p<3;p++)for(int i=0;i<3;i++)printf(" %08x",load(B+64*p+48+4*i));}
  printf(" %08x %08x\n",load(RNG),load(RNG+4));}
}
'''


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def rows_from(rnd):
    """a camera matrix: a look-at at a random place (as the cameras' rows are), or random Euler rows"""
    pos = [rnd.uniform(-2000, 2000), rnd.uniform(-20, 200), rnd.uniform(-2000, 2000)]
    yaw, pitch, roll = rnd.uniform(-math.pi, math.pi), rnd.uniform(-0.6, 0.6), rnd.choice([0.0, rnd.uniform(-0.2, 0.2)])
    cy, sy, cp, sp, cr, sr = math.cos(yaw), math.sin(yaw), math.cos(pitch), math.sin(pitch), math.cos(roll), math.sin(roll)
    c = [sy * cp, sp, cy * cp]
    a0 = [cy, 0.0, -sy]
    b0 = [c[1] * a0[2] - c[2] * a0[1], c[2] * a0[0] - c[0] * a0[2], c[0] * a0[1] - c[1] * a0[0]]
    a = [a0[i] * cr + b0[i] * sr for i in range(3)]
    b = [b0[i] * cr - a0[i] * sr for i in range(3)]
    return [a, b, c, pos]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--count', type=int, default=1500)
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    fn = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', APPLY], capture_output=True, text=True, check=True).stdout.split()[1]
    exe = rd.build(idx, [fn], MAIN.replace('sub_APPLY', fn), a.work, 'camera_shake_apply', image=rd.load_image(a.image))
    rnd = random.Random(10707)
    lines = []
    for _ in range(a.count + a.count // 4):
        dt = rnd.choice([1 / 30, 1 / 60, rnd.uniform(0.001, 0.1)])
        m = rows_from(rnd)
        # the car: behind the camera's look, near it (the 0.02 squared test), or right on it
        back = rnd.choice([rnd.uniform(2, 9), rnd.uniform(0.1, 0.16), 0.0, 0.1414, 0.1415])
        car = [m[3][i] - m[2][i] * back + rnd.choice([0.0, rnd.uniform(-0.5, 0.5)]) * (back > 1) for i in range(3)]
        words = [fb(v) for v in car] + [fb(v) for r in m for v in r]
        for b in range(5):
            st = rnd.choice([0, 0, 1, 2, 2])
            tm = rnd.choice([1.0, rnd.uniform(-0.05, 0.8), dt, dt * 0.5])
            f1 = rnd.choice([3.0, 0.01, rnd.uniform(0, 5)])
            f2 = rnd.choice([0.0, 0.4, 1.0, 2.0, rnd.uniform(0, 3)])
            parts = []
            for p in range(3):
                off = rnd.random() < 0.3
                amp = [0.0 if off else rnd.choice([0.0, rnd.uniform(-0.1, 0.1), rnd.uniform(0, 0.002)]) for _ in range(3)]
                fr = [rnd.choice([50.0, 35.0, 75.0, rnd.uniform(0, 80)]) for _ in range(3)]
                rn = [0.0 if off else rnd.choice([0.0, rnd.uniform(0, 0.01), rnd.uniform(0, 0.0005)]) for _ in range(3)]
                ph = [rnd.choice([0.0, rnd.uniform(0, 50), rnd.uniform(0, 5000)]) for _ in range(3)]
                parts += amp + fr + rn + ph
            words += [st, fb(tm), fb(f1), fb(f2)] + [fb(v) for v in parts]
        lines.append(f'{rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x} {fb(dt):08x} ' + ' '.join('%08x' % w for w in words))
    cases = a.work / 'camera_shake_apply_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines)
    # tracked data must not hold raw-address-looking words
    keep = [l for l in [f'SA {cl} -> {r[2:]}' for cl, r in zip(lines, rows)] if not re.search(r'\b82[0-9a-f]{6}\b', l)][:a.count]
    assert len(keep) == a.count, len(keep)
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(rows), 'cases run,', len(keep), 'kept')


if __name__ == '__main__':
    main()
