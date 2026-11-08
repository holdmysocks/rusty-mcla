"""The replay director's helicopter camera run offline: unchanged generated code of its activate (F-79727, with the polar
activate F-82095, the look-at F-97951, the re-orthonormaliser F-46571 and the zoom timer F-16218 in its closure) and its
per-frame update (F-48462: slerp wrapper F-37101 and slerp, matrix / quaternion conversions) on a guest object, against a
car whose matrix and velocity change every frame. Compiled with -msse4.1 like the recomp (vmsum*fp as dpps).
Object: +16 rows a b c, +64 place, +144 -> the car's matrix, +168 lens, +192 -> the car (+400 velocity, +92 -> +124
gravity scale, set to 1.4), +240 target (0, 0.65, 0), +256 offset (0, 30, 25), +276 angle, +280 / +284 lens range,
+304 velocity, +320 / +324 / +328 match factors, +332 most acceleration, +336 lens goal, +340 lens rate, +344 turn rate,
+348 .. +360 zoom time (mean, deviation, least, most), +364 zoom timer, +368 side. The game's generator object and frame
time (the timer's +8, the scaled frame step) are guest globals; the static-initialised vector constants (xyz mask, (0, 0, 0, 1), zero, +inf) are stored first.
Line:  HELI <lo> <carry> <lens lo> <lens hi> <k0> <k1> <k2> <lens rate> <turn rate> <cut x4> | <n> steps:
       <dt> <car rows a b c place (12)> <car velocity (3)> (step 0 = activate)
       -> per step: rows a b c (9) place (3) velocity (3) lens goal angle zoom side generator lo carry spare-flag spare
(floats as hex bits)
Usage: replay_heli_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

IDS = {'ACT': 'F-79727', 'UPD': 'F-48462'}

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t G=uint32_t(-2105147392)+2176, GDT=uint32_t(-2105737216)+29952+8, V=uint32_t(-2105147392);
 const uint32_t O=0x40010000,CM=0x40010400,CAR=0x40010800,TU=0x40010C00;
 auto f=[](float x){uint32_t u;std::memcpy(&u,&x,4);return u;};
 while(in>>k){rt_reset();
  for(int i=0;i<3;i++)store(V+2384+4*i,0xFFFFFFFFu);store(V+2384+12,0);
  for(int i=0;i<3;i++)store(V+2352+4*i,0);store(V+2352+12,f(1.0f));
  for(int i=0;i<4;i++){store(V+2240+4*i,0);store(V+2528+4*i,0x7F800000u);}
  uint32_t lo,ca,t[11];in>>std::hex>>lo>>ca;for(int i=0;i<11;i++)in>>t[i];
  store(G,lo);store(G+4,ca);store(G+8,0);store(G+12,0);
  for(int j=0;j<0x400;j+=4)store(O+j,0);for(int j=0;j<0x400;j+=4){store(CM+j,0);store(CAR+j,0);store(TU+j,0);}
  store(O+144,CM);store(O+192,CAR);store(CAR+92,TU);store(TU+124,f(1.4f));
  store(O+240,0);store(O+244,f(0.65f));store(O+248,0);store(O+252,0);
  store(O+256,0);store(O+260,f(30.0f));store(O+264,f(25.0f));store(O+268,0);
  store(O+280,t[0]);store(O+284,t[1]);store(O+288,f(7.0f));store(O+320,t[2]);store(O+324,t[3]);store(O+328,t[4]);
  store(O+340,t[5]);store(O+344,t[6]);store(O+348,t[7]);store(O+352,t[8]);store(O+356,t[9]);store(O+360,t[10]);
  int n;in>>std::dec>>n>>std::hex;
  for(int s=0;s<n;s++){uint32_t dt,r[15];in>>dt;for(int i=0;i<15;i++)in>>r[i];store(GDT,dt);
   for(int row=0;row<4;row++){for(int i=0;i<3;i++)store(CM+16*row+4*i,r[3*row+i]);store(CM+16*row+12,row==3?f(1.0f):0);}
   for(int i=0;i<3;i++)store(CAR+400+4*i,r[12+i]);store(CAR+412,0);
   Context c=fresh();c.r3.u32=O;if(s==0)sub_ACT(c,nullptr);else sub_UPD(c,nullptr);
   for(int row=0;row<4;row++)for(int i=0;i<3;i++)printf("%08x ",load(O+16+16*row+4*i));
   for(int i=0;i<3;i++)printf("%08x ",load(O+304+4*i));
   printf("%08x %08x %08x %08x %u %08x %08x %u %08x\n",load(O+168),load(O+336),load(O+276),load(O+364),load(O+368)>>24,load(G),load(G+4),load(G+8)>>24,load(G+12));}
  in>>std::dec;
 }
}
'''


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def rows_of(yaw, pitch, roll):
    """car rows a, b, c of yaw (about y), pitch (about x), roll (about z), each rounded to single"""
    cy, sy, cp, sp, cr, sr = math.cos(yaw), math.sin(yaw), math.cos(pitch), math.sin(pitch), math.cos(roll), math.sin(roll)
    ry = [[cy, 0, -sy], [0, 1, 0], [sy, 0, cy]]
    rx = [[1, 0, 0], [0, cp, sp], [0, -sp, cp]]
    rz = [[cr, sr, 0], [-sr, cr, 0], [0, 0, 1]]
    mul = lambda a, b: [[sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    m = mul(mul(rz, rx), ry)
    return [[f32(v) for v in r] for r in m]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    names = {k: addr(a.tool, v) for k, v in IDS.items()}
    src = MAIN
    for k, v in names.items():
        src = src.replace('sub_' + k + '(', v + '(')
    rd.build(idx, list(names.values()), src, a.work, 'replay_heli', image=rd.load_image(a.image))
    cpp, exe = a.work / 'replay_heli.cpp', a.work / 'replay_heli_sse.exe'
    r = subprocess.run(['g++', '-std=c++17', '-O2', '-msse4.1', '-ffp-contract=off', '-static', '-I' + str(rc.SIMDE), str(cpp), '-o', str(exe)],
                       capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:1500])
    rnd = random.Random(2042)
    lines = []
    STEPS = 25
    for _ in range(120):
        lo_f, hi_f = rnd.choice([(4.0, 20.0), (rnd.uniform(2, 15), rnd.uniform(15, 60))])
        k = rnd.choice([(2.0, 1.0, 1.0), (rnd.uniform(0, 4), rnd.uniform(0, 3), rnd.uniform(0, 3))])
        rates = rnd.choice([(20.0, 5.0), (rnd.uniform(1, 40), rnd.uniform(0.5, 40))])
        cut = rnd.choice([(15.0, 5.0, 5.0, 30.0), (0.3, 0.2, 0.05, 0.6), (rnd.uniform(0, 3), rnd.uniform(0, 1), 0.0, rnd.uniform(0.2, 4))])
        head = [lo_f, hi_f, *k, *rates, *cut]
        pos = [rnd.uniform(-2000, 2000), rnd.uniform(-10, 80), rnd.uniform(-2000, 2000)]
        yaw, pitch, roll = rnd.uniform(-math.pi, math.pi), rnd.uniform(-0.3, 0.3), rnd.uniform(-0.3, 0.3)
        speed = rnd.choice([0.0, rnd.uniform(0, 80)])
        steps = []
        dt = rnd.choice([1 / 30, 1 / 60, rnd.uniform(0.005, 0.1)])
        for s in range(STEPS):
            yaw += rnd.uniform(-0.05, 0.05)
            m = rows_of(yaw, pitch, roll)
            vel = [f32(m[2][i] * -speed + rnd.uniform(-0.5, 0.5)) for i in range(3)]
            pos = [f32(pos[i] + vel[i] * dt) for i in range(3)]
            w = [dt] + [v for r in m for v in r] + pos + vel
            steps.append(' '.join(f'{fb(x):08x}' for x in w))
        lines.append(f'HELI {rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x} ' + ' '.join(f'{fb(x):08x}' for x in head) + f' {STEPS} ' + ' '.join(steps))
    cases = a.work / 'replay_heli_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines) * STEPS, len(rows)
    out = [l + ' -> ' + ' | '.join(r.strip() for r in rows[i * STEPS:(i + 1) * STEPS]) for i, l in enumerate(lines)]
    keep = [l for l in out if not re.search(r'(?<![0-9a-zA-Z])82[0-9a-f]{6}(?![0-9a-zA-Z])', l)]      # tracked data must not hold raw-address-looking words
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(keep), 'of', len(out), 'sequences')


if __name__ == '__main__':
    main()
