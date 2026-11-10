"""The top GPS arrow's frame F-30300 run offline as generated: the target taken into the car's frame by the inverse frame
F-30106 (the car's own rows where the inverse is refused), z = -(that), x = z x up, y = x x z, each made unit by the vector
unit, then gpsArrowRotation = F-32060((0, 0, -1), local) x -57.29578 handed to the movie's float setter F-66095 (stubbed: it
records the value). The control object: +208 the target, +612 no pointer; the frame goes to +((index + 4) x 64), the index
word the game keeps at (0x8289 << 16) - 17936 set to 0. The runtime vector constants are set as the game sets them ((0,0,0,1),
the xyz mask, +inf, zeros). The vector unit flushes denormals (the host's FTZ / DAZ while it runs).
Line:  <rows a b c place (12)> <target (3)> -> <x (3)> <y (3)> <z (3)> <rotation>   (every float as its 32 bits, in decimal)
Usage: arrow_frame_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t C=0x40010000,M=0x40011000,R=0x40012000;
 const uint32_t V=uint32_t(-2105147392),IDX=uint32_t(-2104950784)-17936;
 uint32_t m[12],t[3];
 while(in>>m[0]){for(int i=1;i<12;i++)in>>m[i];for(auto&x:t)in>>x;rt_reset();
  for(uint32_t a=C;a<0x40013000;a+=4)store(a,0);
  for(int i=0;i<4;i++){store(V+2352+4*i,i==3?0x3F800000u:0);store(V+2384+4*i,i<3?0xFFFFFFFFu:0);store(V+2528+4*i,0x7F800000u);store(V+2240+4*i,0);}
  store(IDX,0);
  for(int r=0;r<4;r++){for(int i=0;i<3;i++)store(M+16*r+4*i,m[3*r+i]);store(M+16*r+12,r==3?0x3F800000u:0);}
  for(int i=0;i<3;i++)store(C+208+4*i,t[i]);store(C+208+12,0x3F800000u);
  store(C+612,0);store(C+192,R);store(R+0x100,0x7FC00001u);
  Context cx=fresh();cx.r3.u32=C;cx.r4.u32=M;sub_FRAME(cx,nullptr);
  for(int r=0;r<3;r++)for(int i=0;i<3;i++)printf("%u ",load(C+256+16*r+4*i));
  printf("%u\n",load(R+0x100));
 }
}
'''


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    frame, setter = addr('F-30300'), addr('F-66095')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    # (the movie's float setter: the value it is handed, at R + 0x100)
    stubs = {setter: 'uint32_t b;float f=float(ctx.f1.f64);memcpy(&b,&f,4);store(0x40012100u,b);'}
    src = MAIN.replace('sub_FRAME(', frame + '(')
    exe = rd.build(idx, [frame], src, a.work, 'arrow_frame', image=img, stubs=stubs)
    rnd = random.Random(30300)
    lines = []
    for n in range(3000):
        q = [rnd.gauss(0, 1) for _ in range(4)]
        l = math.sqrt(sum(x * x for x in q))
        x, y, z, w = (v / l for v in q)
        if rnd.random() < 0.5:
            # (a car on the level: a turn about up only, a little pitch / roll now and then)
            x = z = 0.0
            if rnd.random() < 0.5:
                x, z = rnd.uniform(-0.05, 0.05), rnd.uniform(-0.05, 0.05)
            l = math.sqrt(x * x + y * y + z * z + w * w)
            x, y, z, w = x / l, y / l, z / l, w / l
        m = [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)], [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
             [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]
        if n % 50 == 7:
            m = [[1.0, 0, 0], [0, 1.0, 0], [0, 0, 1.0]]
        if n % 97 == 3:
            m = [[0.0, 0, 0], [0, 0, 0], [0, 0, 0]]
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 100), rnd.uniform(-3000, 3000)]
        kind = rnd.random()
        if kind < 0.6:
            t = [place[0] + rnd.uniform(-400, 400), place[1] + rnd.uniform(-30, 30), place[2] + rnd.uniform(-400, 400)]
        elif kind < 0.8:
            t = [place[0] + rnd.uniform(-3, 3), place[1] + rnd.uniform(-1, 1), place[2] + rnd.uniform(-3, 3)]
        elif kind < 0.9:
            t = [place[0], place[1] + rnd.choice([0.0, 5.0, -5.0]), place[2]]
        else:
            t = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 100), rnd.uniform(-3000, 3000)]
        if n % 50 == 7 and rnd.random() < 0.5:
            t = list(place)
        lines.append(' '.join(str(fb(v)) for v in [c for r in m for c in r] + place + t))
    cases = a.work / 'arrow_frame_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines), (len(rows), len(lines))
    a.out.write_text(''.join(l + ' -> ' + r + '\n' for l, r in zip(lines, rows)))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
