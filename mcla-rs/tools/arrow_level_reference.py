"""The top GPS arrow's level frame as F-32331 builds it (frame 6) and F-30300 on it, run offline: F-32331's stores into the
frame (row 1 = (0, 1, 0), row 2 = the car matrix's z row with its y (+36) set to 0, row 3 = the car's place), then the
orthonormaliser F-49999 as generated, then F-32331's own row 0 (the block after the call, transcribed: row1 x (0 - row2) by
the vector unit's products, each rounded, the difference rounded), then F-30300 as generated (arrow_frame_reference.py has
its set-up). The vector unit flushes denormals (the host's FTZ / DAZ while it runs).
Line:  <car rows a b c (9)> <place (3)> <target (3)> -> <frame rows 0 1 2 (9)> <x (3)> <y (3)> <z (3)> <rotation>   (floats as their 32 bits)
Usage: arrow_level_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
static float bf(uint32_t b){float f;memcpy(&f,&b,4);return f;}
static uint32_t fbits(float f){uint32_t b;memcpy(&b,&f,4);return b;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t C=0x40010000,M=0x40011000,R=0x40012000;
 const uint32_t V=uint32_t(-2105147392),IDX=uint32_t(-2104950784)-17936;
 uint32_t m[12],t[3];
 while(in>>m[0]){for(int i=1;i<12;i++)in>>m[i];for(auto&x:t)in>>x;rt_reset();
  for(uint32_t a=C;a<0x40013000;a+=4)store(a,0);
  for(int i=0;i<4;i++){store(V+2352+4*i,i==3?0x3F800000u:0);store(V+2384+4*i,i<3?0xFFFFFFFFu:0);store(V+2528+4*i,0x7F800000u);store(V+2240+4*i,0);}
  store(IDX,0);
  // F-32331's stores: row 1 (0, 1, 0), row 2 the car's z row, +36 = 0, row 3 the place
  store(M+16,0);store(M+20,0x3F800000u);store(M+24,0);store(M+28,0);
  for(int i=0;i<3;i++)store(M+32+4*i,m[6+i]);store(M+44,0);store(M+36,0);
  for(int i=0;i<3;i++)store(M+48+4*i,m[9+i]);store(M+60,0x3F800000u);
  {Context cx=fresh();cx.r3.u32=M;sub_ORTHO(cx,nullptr);}
  // F-32331's row 0: v34 = perm135(row1) (z,x,y), v33 = perm99(-row2) (y,z,x), v0 = perm99(row1), v13 = perm135(-row2);
  // v11 = -(v0 v13 - v34 v33), each product rounded
  {float r1[3],n2[3];for(int i=0;i<3;i++){r1[i]=bf(load(M+16+4*i));n2[i]=0.0f-bf(load(M+32+4*i));}
   float a[3]={r1[2],r1[0],r1[1]},b[3]={n2[1],n2[2],n2[0]},c[3]={r1[1],r1[2],r1[0]},d[3]={n2[2],n2[0],n2[1]};
   for(int i=0;i<3;i++){volatile float p=a[i]*b[i];volatile float q=c[i]*d[i];volatile float s=q-p;store(M+4*i,fbits(-s));}}
  for(int i=0;i<3;i++)store(C+208+4*i,t[i]);store(C+208+12,0x3F800000u);
  store(C+612,0);store(C+192,R);store(R+0x100,0x7FC00001u);
  Context cx=fresh();cx.r3.u32=C;cx.r4.u32=M;sub_FRAME(cx,nullptr);
  for(int r=0;r<3;r++)for(int i=0;i<3;i++)printf("%u ",load(M+16*r+4*i));
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
    frame, setter, ortho = addr('F-30300'), addr('F-66095'), addr('F-49999')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    # (the movie's float setter: the value it is handed, at R + 0x100)
    stubs = {setter: 'uint32_t b;float f=float(ctx.f1.f64);memcpy(&b,&f,4);store(0x40012100u,b);'}
    src = MAIN.replace('sub_FRAME(', frame + '(').replace('sub_ORTHO(', ortho + '(')
    exe = rd.build(idx, [frame, ortho], src, a.work, 'arrow_level', image=img, stubs=stubs)
    rnd = random.Random(32331)
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
        lines.append(' '.join(str(fb(v)) for v in [c for r in m for c in r] + place + t))  # (car rows, place, target)
    cases = a.work / 'arrow_level_cases.txt'
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
