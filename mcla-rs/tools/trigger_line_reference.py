"""The line trigger's check F-37517 run offline as generated: the owner test (+48: 0 = anyone, else the entity's id), the move from the
entity's previous place (+400) to its matrix place (F-35658's +48, stubbed to a fixed matrix) against the line +64 -> +80 in x/z
through F-95591 / F-66886 with the one-way byte +100, then the height window (the move's height at the meeting point below the
line's + the height +96 and not below the line's - 2) and the trigger's vt+28 (a fake: it records that it ran);
it returns 1 only when that ran.
Line:  <a (3)> <b (3)> <height> <one way> <owner> <id> <prev (3)> <cur (3)> -> <return> <fired>   (floats as their 32 bits, decimal)
Usage: trigger_line_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

FIRE = 0x7F000010
MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t T=0x40010000,V=0x40011000,E=0x40012000,M=0x40013000,F=0x40014000;
 uint32_t a[3],b[3],h,one,owner,id,p[3],c[3];
 while(in>>a[0]){in>>a[1]>>a[2];for(auto&x:b)in>>x;in>>h>>one>>owner>>id;for(auto&x:p)in>>x;for(auto&x:c)in>>x;rt_reset();
  for(uint32_t x=T;x<0x40014100;x+=4)store(x,0);
  store(T,V);store(V+28,0x7F000010u);store(T+48,owner);
  for(int i=0;i<3;i++){store(T+64+4*i,a[i]);store(T+80+4*i,b[i]);store(E+400+4*i,p[i]);store(M+48+4*i,c[i]);}
  store(T+96,h);store(T+100,one<<24);
  Context cx=fresh();cx.r3.u32=T;cx.r4.u32=E;cx.r5.u32=id;sub_CHECK(cx,nullptr);
  printf("%u %u\n",cx.r3.u32&255,load(F));
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
    check, place = addr('F-37517'), addr('F-35658')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [check], MAIN.replace('sub_CHECK(', check + '('), a.work, 'trigger_line', image=img,
                   stubs={place: 'ctx.r3.u32=0x40013000u;'}, fakes={FIRE: 'store(0x40014000u,1);'})
    rnd = random.Random(37517)
    lines = []
    for n in range(4000):
        cx, cz, cy = rnd.uniform(-3000, 3000), rnd.uniform(-3000, 3000), rnd.uniform(-20, 100)
        L = rnd.uniform(0.0, 0.00002) if rnd.random() < 0.08 else rnd.uniform(2, 30)
        ang = rnd.uniform(-3.2, 3.2)
        if rnd.random() < 0.15:
            ang = rnd.choice([0.0, 1.5707964, 3.1415927, -1.5707964])
        ca, sa = L * 0.5 * __import__('math').cos(ang), L * 0.5 * __import__('math').sin(ang)
        A = [cx - ca, cy + rnd.uniform(-1, 1), cz - sa]
        B = [cx + ca, cy + rnd.uniform(-1, 1), cz + sa]
        h = rnd.choice([20.0, 5.0, rnd.uniform(0, 30)])
        one = rnd.choice([0, 1, 1])
        idv = rnd.randint(1, 900)
        owner = rnd.choice([0, 0, idv, idv + 1])
        # (a move through a point near the line, either way, at a height around it; now and then along it or still)
        tt = rnd.uniform(-0.1, 1.1)
        m = [A[0] + (B[0] - A[0]) * tt, cy, A[2] + (B[2] - A[2]) * tt]
        if rnd.random() < 0.25:
            m = [cx + rnd.uniform(-L, L) * 0.6, cy, cz + rnd.uniform(-L, L) * 0.6]
        step = rnd.choice([rnd.uniform(0.05, 3), rnd.uniform(0, 40)])
        da = rnd.uniform(-3.2, 3.2)
        dx, dz = step * __import__('math').cos(da), step * __import__('math').sin(da)
        k = rnd.random()
        if k < 0.1:
            dx, dz = ca * 0.1, sa * 0.1
        elif k < 0.13:
            dx = dz = 0.0
        y0 = cy + rnd.choice([0.0, rnd.uniform(-4, 25), rnd.uniform(-4, 25), h - 0.001, h + 0.5, -2.0, -1.999, -2.5])
        dy = rnd.choice([0.0, rnd.uniform(-1, 1)])
        P = [m[0] - dx * 0.5, y0 - dy, m[2] - dz * 0.5]
        C = [m[0] + dx * 0.5, y0 + dy, m[2] + dz * 0.5]
        if rnd.random() < 0.05:
            P, C = [cx, y0, cz], [cx + dx, y0, cz + dz]
        lines.append(' '.join(str(fb(v)) for v in A + B + [h]) + f' {one} {owner} {idv} ' + ' '.join(str(fb(v)) for v in P + C))
    cases = a.work / 'trigger_line_cases.txt'
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
