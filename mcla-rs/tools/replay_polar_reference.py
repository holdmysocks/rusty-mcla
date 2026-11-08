"""The replay director's polar camera update F-72161 run offline: unchanged generated code with its world collision F-67414, the
rotations F-43547 / F-57591, the look-at F-97951 and the re-orthonormaliser F-46571, on a polar camera object (its real vtable,
found by class name) over a car with any matrix. The sweep F-58512 is stubbed: it logs start, end and radius and answers from the
case (0: no touch; 1: a touch whose contact and normal are written at its result +16 / +32). The push-out to the car's own bound
F-38041 runs unchanged; its query calls are stubbed: the query constructor F-93477 and the release F-13813 / F-47121 do nothing, the
directed setup F-13494 logs start, end (its r4 block) and radius and keeps the result pointer, the instance frame F-56283 and the
world conversion F-77105 do nothing (the case's contact and normal are already in the world), the bound test F-35029 answers from the
case (a touch writes the case's second contact and normal at the result +16 / +32). The viewport (+152) holds near +704 and F-90812's tangents +712 / +716.
Line:  POL <lens> <aspect> <pitch> <yaw> <car rows a b c place (12)> <touch 0/1> <contact x3> <normal x3>
       <own touch 0/1> <contact x3> <normal x3>
       -> [W<ax>,<ay>,<az>,<bx>,<by>,<bz>,<r> | O<ax>,..,<r>, each followed by a space] <rows a b c (9)> <place (3)> <+156>
(floats as hex bits; the rows are the ones the camera shows: +80 after a touch, else +16)
Usage: replay_polar_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = {'UPD': 'F-72161', 'SWEEP': 'F-58512', 'QCT': 'F-93477', 'QDIR': 'F-13494', 'QINST': 'F-56283', 'QTEST': 'F-35029',
       'QWORLD': 'F-77105', 'QREL1': 'F-13813', 'QREL2': 'F-47121'}

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t V=uint32_t(-2105147392);
 const uint32_t PO=0x40010000,CM=0x40010800,VP=0x40012000,QU=0x40014000;
 auto f=[](float x){uint32_t u;std::memcpy(&u,&x,4);return u;};
 auto g=[](uint32_t u){float x;std::memcpy(&x,&u,4);return x;};
 while(in>>k){rt_reset();
  for(int i=0;i<3;i++)store(V+2384+4*i,0xFFFFFFFFu);store(V+2384+12,0);
  for(int i=0;i<3;i++)store(V+2352+4*i,0);store(V+2352+12,f(1.0f));
  for(int i=0;i<4;i++){store(V+2240+4*i,0);store(V+2528+4*i,0x7F800000u);}
  for(uint32_t a=0x40010000;a<0x40014400;a+=4)store(a,0);
  uint32_t lens,asp,pi,ya,m[12],hit,c[3],nm[3],oh,oc[3],on[3];
  in>>std::hex>>lens>>asp>>pi>>ya;for(auto&x:m)in>>x;in>>hit;for(auto&x:c)in>>x;for(auto&x:nm)in>>x;
  in>>oh;for(auto&x:oc)in>>x;for(auto&x:on)in>>x;in>>std::dec;
  store(QU+64,oh);for(int i=0;i<3;i++){store(QU+80+4*i,oc[i]);store(QU+96+4*i,on[i]);}
  store(QU,hit);for(int i=0;i<3;i++){store(QU+16+4*i,c[i]);store(QU+32+4*i,nm[i]);}
  for(int r=0;r<4;r++){for(int i=0;i<3;i++)store(CM+16*r+4*i,m[3*r+i]);store(CM+16*r+12,r==3?f(1.0f):0);}
  float t=float(std::tan(double(g(lens)*g(0x3c0efa35u))));store(VP+696,lens);store(VP+704,f(0.2f));store(VP+712,f(t));store(VP+716,f(t*g(asp)));
  store(PO,VTP);store(PO+144,CM);store(PO+152,VP);store(PO+244,f(0.65f));store(PO+260,f(1.0f));store(PO+264,f(7.0f));
  store(PO+272,pi);store(PO+276,ya);store(PO+168,f(47.0f));store(PO+192,0x40013000u);store(0x40013000u+28,0x40013100u);store(0x40013100u+4,0x40013200u);store(0x40013200u+12,0x40013300u);   // the vehicle (+28: its instance, left out of the sweep)

  Context cx=fresh();cx.r3.u32=PO;sub_UPD(cx,nullptr);
  uint32_t M=(load(PO+156)>>24)?PO+80:PO+16;
  for(int r=0;r<4;r++)for(int i=0;i<3;i++)printf("%08x ",load(M+16*r+4*i));
  printf("%u\n",load(PO+156)>>24);
 }
}
'''

STUBS = {
    'SWEEP': 'printf("W%08x,%08x,%08x,%08x,%08x,%08x,%08x ",load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r5.u32),'
             'load(ctx.r5.u32+4),load(ctx.r5.u32+8),[&]{float x=float(ctx.f1.f64);uint32_t u;std::memcpy(&u,&x,4);return u;}());'
             'uint32_t h=load(0x40014000u);if(h){for(int i=0;i<8;i++)store(ctx.r7.u32+16+4*i,load(0x40014010u+4*i));}ctx.r3.u64=h;',
    'QDIR': 'printf("O%08x,%08x,%08x,%08x,%08x,%08x,%08x,%u ",load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r4.u32+16),'
            'load(ctx.r4.u32+20),load(ctx.r4.u32+24),[&]{float x=float(ctx.f1.f64);uint32_t u;std::memcpy(&u,&x,4);return u;}(),ctx.r7.u32);'
            'store(0x40014070u,ctx.r6.u32);',
    'QTEST': 'uint32_t h=load(0x40014040u),r=load(0x40014070u);if(h){for(int i=0;i<8;i++)store(r+16+4*i,load(0x40014050u+4*i));}'
             'if(ctx.r4.u32!=0x40013300u||ctx.r5.u32!=0x40013100u)printf("Q? ");ctx.r3.u64=h;',
    'QCT': '', 'QINST': '', 'QWORLD': '', 'QREL1': '', 'QREL2': '',
}
def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


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
    names = {k: addr(a.tool, v) for k, v in IDS.items()}
    vt = rd.vtables_of(img, 'mcReplayCameraPolar')[0][0]
    src = MAIN.replace('VTP', '0x%08Xu' % vt)
    for k, v in names.items():
        src = src.replace('sub_' + k + '(', v + '(')
    stubs = {names[k]: b for k, b in STUBS.items()}
    exe = rd.build(idx, [names['UPD']], src, a.work, 'replay_polar', image=img, stubs=stubs)
    rnd = random.Random(2045)
    f32 = lambda x: struct.unpack('>f', struct.pack('>f', x))[0]
    lines = []
    for _ in range(6000):
        q = [rnd.gauss(0, 1) for _ in range(4)]
        n = math.sqrt(sum(x * x for x in q))
        x, y, z, w = (v / n for v in q)
        if rnd.random() < 0.5:
            x = z = 0.0
            n = math.sqrt(y * y + w * w)
            y, w = y / n, w / n
        m = [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)], [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
             [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 100), rnd.uniform(-3000, 3000)]
        lens = rnd.choice([65.0, 47.0, rnd.uniform(20, 90)])
        asp = rnd.choice([16 / 9, rnd.uniform(1, 2.5)])
        pitch = rnd.choice([0.0, 1.2566371, rnd.uniform(0, 1.26)])
        yaw = rnd.uniform(-20, 20)
        hit = rnd.choice([0, 1, 1])
        c = [place[0] + rnd.uniform(-8, 8), place[1] + rnd.uniform(-2, 8), place[2] + rnd.uniform(-8, 8)]
        nv = [rnd.gauss(0, 1) for _ in range(3)]
        nl = math.sqrt(sum(v * v for v in nv))
        nv = [v / nl for v in nv]
        own = rnd.choice([0, 1])
        c2 = [place[0] + rnd.uniform(-3, 3), place[1] + rnd.uniform(-1, 2), place[2] + rnd.uniform(-3, 3)]
        n2 = [rnd.gauss(0, 1) for _ in range(3)]
        nl = math.sqrt(sum(v * v for v in n2))
        n2 = [v / nl for v in n2]
        vals = [lens, asp, pitch, yaw] + [v for r in m for v in r] + place
        lines.append('POL ' + ' '.join(f'{fb(v):08x}' for v in vals) + f' {hit} ' + ' '.join(f'{fb(v):08x}' for v in c + nv)
                     + f' {own} ' + ' '.join(f'{fb(v):08x}' for v in c2 + n2))
    cases = a.work / 'replay_polar_cases.txt'
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
