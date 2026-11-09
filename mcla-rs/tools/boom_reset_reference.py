"""The boom camera's reset F-83404 (camDynamicBoomCS slot 9) run offline: unchanged generated code with its base reset F-74452, the
frame F-90220 (at the snap share 1), the velocity start F-17419 and the look-at, on a camera object over a target with any matrix,
wheels, velocity and spin. The camera (+144 the followed matrix = target +144, +192 the target, +240 the tune, +208 looking back,
+668 the fixed mount) and its tune (the reset's and the frame's fields at their offsets: Center +16, Offset +32, Focus +48, RevOffset
+80, RevFocus +96, ApproachRate +128, CollideType +132, OrientationFactor / OrientationFilter / DirectionFilter +212 / +216 / +220,
SSFovMin +244; the shift curve at +448 a stub object whose slot 7 does nothing). The target +8 is the chassis: +24 the wheel count,
+28 the wheels (+529 contact byte, +336 normal, +368 place, +320 a third vector), +128 the matrix object (+272 velocity, +288 spin),
+224 the speed, +592 the state word. The frame time is the game's global (+8 of the time block), the camera's +184 clear. The runtime vector constants are set as
the game sets them ((0,0,0,1), the xyz mask, +inf, zeros; the frame's world up (0,1,0)).
Line:  RST <rev> <fixed> <dt> <tune: center offset focus revoffset revfocus (15) rate ofac ofil dfil ssfovmin>
       <rows a b c place (12)> <velocity (3)> <spin (3)> <speed> <state> <count> <per wheel: contact, place (3), normal (3)>
       -> <up (3)> <dir (3)> <tumble> <vel (3)> <acc (3)> <ang (3)> <ang_acc (3)> <off (3)> <foc (3)> <roll> <world (3)> <rate>
          <gz> <shift> <shift value> <armed> <lens> <snap> <ability lens +224>
(floats as hex bits)
Usage: boom_reset_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

FAKE = 0x7F000100
MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t GDT=uint32_t(-2105737216)+29952+8;
 const uint32_t C=0x40010000,T=0x40010800,S=0x40010C00,M=0x40011000,TU=0x40011400,CV=0x40011C00,VT=0x40011D00,W=0x40012000,WP=0x40011E00;
 while(in>>k){rt_reset();
  for(uint32_t a=C;a<0x40013000;a+=4)store(a,0);
  uint32_t rev,fix,dt,tu[20],m[12],v[3],sp[3],spd,st,n;
  in>>rev>>fix>>std::hex>>dt;for(auto&x:tu)in>>x;for(auto&x:m)in>>x;for(auto&x:v)in>>x;for(auto&x:sp)in>>x;in>>spd>>std::dec>>st>>n;
  store(GDT,dt);
  // (the runtime-initialised vector constants: (0,0,0,1), the xyz mask, +inf, the zeros, and the frame's world up (0,1,0) -
  // the value the recorded frame calls match, boomcam BOOM_UP)
  const uint32_t V=uint32_t(-2105147392);
  for(int i=0;i<4;i++){store(V+2352+4*i,i==3?0x3F800000u:0);store(V+2384+4*i,i<3?0xFFFFFFFFu:0);store(V+2528+4*i,0x7F800000u);
   store(V+2240+4*i,0);store(V+2272+4*i,0);store(V+2288+4*i,i==1?0x3F800000u:0);}
  const int VO[5]={16,32,48,80,96};
  for(int q=0;q<5;q++)for(int i=0;i<3;i++)store(TU+VO[q]+4*i,tu[3*q+i]);
  store(TU+128,tu[15]);store(TU+132,2);store(TU+212,tu[16]);store(TU+216,tu[17]);store(TU+220,tu[18]);store(TU+244,tu[19]);
  store(TU+448,VT);store(VT+28,FAKEA);   // (the curve object lives in the tune at +448)
  for(int r=0;r<4;r++){for(int i=0;i<3;i++)store(T+144+16*r+4*i,m[3*r+i]);store(T+144+16*r+12,r==3?0x3F800000u:0);}
  store(T+8,S);store(S+24,n);store(S+28,WP);store(S+128,M);store(S+224,spd);store(S+592,st);
  for(int i=0;i<3;i++){store(M+272+4*i,v[i]);store(M+288+4*i,sp[i]);}
  for(uint32_t w=0;w<n;w++){uint32_t c,p[3],q[3],e[3];in>>c>>std::hex;for(auto&x:p)in>>x;for(auto&x:q)in>>x;for(auto&x:e)in>>x;in>>std::dec;
   uint32_t a=W+0x300*w;store(WP+4*w,a);store(a+528,c<<16);for(int i=0;i<3;i++){store(a+368+4*i,p[i]);store(a+336+4*i,q[i]);store(a+320+4*i,e[i]);}}
  store(C+144,T+144);store(C+192,T);store(C+240,TU);store(C+208,rev);store(C+664,0);store(C+668,fix<<24);
  for(int i=0;i<3;i++){store(C+464+4*i,0x41200000u);store(C+416+4*i,0x40A00000u);}   // (stale state the reset must overwrite)
  store(C+400,0x3F000000u);store(C+168,0x42000000u);store(C+224,0x41200000u);store(C+656,0x41A00000u);
  Context cx=fresh();cx.r3.u32=C;sub_RST(cx,nullptr);
  const int OUT3[]={272,384},OUT1a[]={400};
  auto p3=[&](uint32_t o){for(int i=0;i<3;i++)printf("%08x ",load(C+o+4*i));};
  p3(272);p3(384);printf("%08x ",load(C+400));p3(464);p3(480);p3(496);p3(512);p3(416);p3(432);printf("%08x ",load(C+448));p3(64);
  printf("%08x %08x %08x %08x %u %08x %u %08x\n",load(C+656),load(C+568),load(C+596),load(C+600),load(C+608)>>24,load(C+168),load(C+664)>>24,load(C+224));
 }
}
'''


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(rnd):
    v = [rnd.gauss(0, 1) for _ in range(3)]
    n = math.sqrt(sum(x * x for x in v))
    return [x / n for x in v]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    name = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', 'F-83404'], capture_output=True, text=True, check=True).stdout.split()[1]
    src = MAIN.replace('sub_RST(', name + '(').replace('FAKEA', '0x%08Xu' % FAKE)
    exe = rd.build(idx, [name], src, a.work, 'boom_reset', image=img, fakes={FAKE: ''})
    rnd = random.Random(83404)
    lines = []
    for _ in range(4000):
        q = [rnd.gauss(0, 1) for _ in range(4)]
        n = math.sqrt(sum(x * x for x in q))
        x, y, z, w = (v / n for v in q)
        if rnd.random() < 0.6:
            x = z = 0.0
            n = math.sqrt(y * y + w * w)
            y, w = y / n, w / n
        m = [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)], [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
             [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 100), rnd.uniform(-3000, 3000)]
        tune = [rnd.uniform(-1, 1) for _ in range(3)] + [rnd.uniform(-2, 2), rnd.uniform(0, 3), rnd.uniform(-8, 8)] \
            + [rnd.uniform(-1, 1), rnd.uniform(0, 2), rnd.uniform(-2, 2)] + [rnd.uniform(-2, 2), rnd.uniform(0, 3), rnd.uniform(4, 8)] \
            + [rnd.uniform(-1, 1), rnd.uniform(0, 2), rnd.uniform(-2, 2)]
        tune += [rnd.choice([0.0, 8.0, rnd.uniform(0, 40)]), rnd.uniform(0, 1), rnd.uniform(0, 1), rnd.uniform(0, 1), rnd.uniform(20, 70)]
        vel = [rnd.uniform(-60, 60), rnd.uniform(-10, 10), rnd.uniform(-60, 60)] if rnd.random() < 0.8 else [0.0, 0.0, 0.0]
        spin = [rnd.uniform(-3, 3) for _ in range(3)]
        speed = math.sqrt(sum(v * v for v in vel)) if rnd.random() < 0.8 else rnd.uniform(0, 30)
        state = rnd.choice([0, 0, 0x10, rnd.getrandbits(32)])
        count = rnd.choice([4, 4, 4, 2, 0])
        dt = rnd.choice([1 / 30, 1 / 60, rnd.uniform(0.001, 0.1)])
        wheels = []
        for i in range(count):
            c = rnd.choice([1, 1, 1, 0])
            p = [place[0] + rnd.uniform(-2, 2), place[1] + rnd.uniform(-1, 0.5), place[2] + rnd.uniform(-2, 2)]
            nv = unit(rnd) if rnd.random() < 0.3 else [rnd.uniform(-0.3, 0.3), 1.0, rnd.uniform(-0.3, 0.3)]
            l = math.sqrt(sum(v * v for v in nv))
            nv = [v / l for v in nv]
            e = [rnd.uniform(-5, 5) for _ in range(3)]
            wheels.append(f'{c} ' + ' '.join(f'{fb(v):08x}' for v in p + nv + e))
        rev, fix = rnd.choice([0, 1]), rnd.choice([0, 0, 1])
        lines.append(f'RST {rev} {fix} {fb(dt):08x} ' + ' '.join(f'{fb(v):08x}' for v in tune + [c for r in m for c in r] + place + vel + spin + [speed])
                     + f' {state} {count} ' + ' '.join(wheels))
    cases = a.work / 'boom_reset_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines), (len(rows), len(lines))
    out = [l + ' -> ' + r for l, r in zip(lines, rows)]
    keep = [l for l in out if not re.search(r'(?<![0-9a-zA-Z])82[0-9a-f]{6}(?![0-9a-zA-Z])', l)]      # tracked data must not hold raw-address-looking words
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(keep), 'of', len(out), 'cases')


if __name__ == '__main__':
    main()
