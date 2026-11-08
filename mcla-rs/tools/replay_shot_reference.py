"""The replay director's shot test F-51014 run offline: unchanged generated code on a set-up replay camera whose fixed and helicopter
cameras are guest objects with their real vtables (found by class name; slot 10 = their updates F-77143 / F-48462, both checked apart).
The three probes are stubbed: they log what they get and answer from the case's list (1 = touches / hits):
  P  the ball at a point F-87651   (point, radius, include mask)
  W  the directed sweep F-58512    (start, end, radius, include mask)
  H  the helicopter's sweep F-80278 (start, end, radius, include mask)
For the polar state (19) its update F-72161 and push-out F-38041 run unchanged; the push-out's query calls are stubbed as in
replay_polar_reference.py, its bound test answering from the same list:
  O  the push-out's sweep on the car's bound (setup F-13494: start, end, radius, directed flag)
A touch of W (in the polar state) and of O writes contact = the sweep's start and normal (0, 1, 0) at the result +16 / +32.
The car's bound getter F-98453 is stubbed to a block whose +20 is the car's top. The replay's car +48 -> its vehicle: identity rows at +144 (row 3
the place), velocity +400, +8 -> the sim with the speed at +224 (the include mask's test); the helicopter's +192 is the same vehicle. The viewport (+152 of each camera) holds near +704 and the tangents
+712 / +716 made from the lens and aspect as F-90812 makes them (tan through the host's double tangent, rounded to single).
Set-up: the mount setter F-31661 for the fixed states, the helicopter's activate F-79727 (generator from the case) for state 18, the
polar camera's activate F-82095 for state 19.
Line:  SHOT <state> <ahead> <lens> <aspect> <car place x3> <car velocity x3> <top> <speed> <cut x4> <mounts x18> <lo> <carry>
       <heli: lens lo hi, k0 k1 k2, lens rate, turn rate, cut x4> <dt> <n answers> <answers>
       -> [P<x>,<y>,<z>,<r>,<mask> | W/H<ax>,<ay>,<az>,<bx>,<by>,<bz>,<r>,<mask>, each followed by a space] <result> <generator lo> <carry>
          <heli lens> <heli zoom>
(floats as hex bits)
Usage: replay_shot_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = {'SHOT': 'F-51014', 'BALL': 'F-87651', 'SWEEP': 'F-58512', 'HSWEEP': 'F-80278', 'BOUND': 'F-98453', 'MNT': 'F-31661',
       'HACT': 'F-79727', 'PACT': 'F-82095', 'QCT': 'F-93477', 'QDIR': 'F-13494', 'QINST': 'F-56283', 'QTEST': 'F-35029',
       'QWORLD': 'F-77105', 'QREL1': 'F-13813', 'QREL2': 'F-47121'}

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t G=uint32_t(-2105147392)+2176, GDT=uint32_t(-2105737216)+29952+8, V=uint32_t(-2105147392);
 const uint32_t D=0x40010000,CM=0x40010800,CAR=0x40010C00,TU=0x40011000,FX=0x40011400,HE=0x40011C00,VP=0x40012000,CH=0x40012800,BD=0x40012C00,QU=0x40014000;
 auto f=[](float x){uint32_t u;std::memcpy(&u,&x,4);return u;};
 auto g=[](uint32_t u){float x;std::memcpy(&x,&u,4);return x;};
 while(in>>k){rt_reset();
  for(int i=0;i<3;i++)store(V+2384+4*i,0xFFFFFFFFu);store(V+2384+12,0);
  for(int i=0;i<3;i++)store(V+2352+4*i,0);store(V+2352+12,f(1.0f));
  for(int i=0;i<4;i++){store(V+2240+4*i,0);store(V+2528+4*i,0x7F800000u);}
  for(uint32_t a=0x40010000;a<0x40014400;a+=4)store(a,0);
  uint32_t st,ah,lens,asp,pos[3],vel[3],top,spd,cut[4],mo[18],lo,ca,ht[11],dt;int n;
  in>>std::hex>>st>>ah>>lens>>asp;for(auto&x:pos)in>>x;for(auto&x:vel)in>>x;in>>top>>spd;for(auto&x:cut)in>>x;for(auto&x:mo)in>>x;
  in>>lo>>ca;for(auto&x:ht)in>>x;in>>dt>>std::dec>>n;store(QU,0);store(QU+4,n);for(int i=0;i<n;i++){int b;in>>b;store(QU+8+4*i,b);}
  store(G,lo);store(G+4,ca);store(G+8,0);store(G+12,0);store(GDT,dt);
  for(int r=0;r<3;r++)store(CM+16*r+4*r,f(1.0f));for(int i=0;i<3;i++)store(CM+48+4*i,pos[i]);store(CM+60,f(1.0f));
  // the replay's car (+48 -> its vehicle: +8 -> the sim, speed +224; +144 matrix; +400 velocity; +92 -> tune), the helicopter's +192
  for(int i=0;i<3;i++)store(CH+400+4*i,vel[i]);store(CH+92,TU);store(TU+124,f(1.4f));
  for(int i=0;i<64;i+=4)store(CH+144+i,load(CM+i));store(CAR+48,CH);store(CH+8,BD+0x100);store(BD+0x100+224,spd);
  store(BD+20,top);
  // the viewport (F-90812's tangents)
  float t=float(std::tan(double(g(lens)*g(0x3c0efa35u))));store(VP+696,lens);store(VP+704,f(0.2f));store(VP+712,f(t));store(VP+716,f(t*g(asp)));
  store(HE,VTH);store(HE+144,CM);store(HE+152,VP);store(HE+192,CH);store(HE+244,f(0.65f));store(HE+260,f(30.0f));store(HE+264,f(25.0f));
  store(HE+280,ht[0]);store(HE+284,ht[1]);store(HE+288,f(7.0f));store(HE+320,ht[2]);store(HE+324,ht[3]);store(HE+328,ht[4]);
  store(HE+340,ht[5]);store(HE+344,ht[6]);for(int i=0;i<4;i++)store(HE+348+4*i,ht[7+i]);
  store(FX,VTF);store(FX+144,CM);store(FX+152,VP);
  const uint32_t PO=0x40013000;store(PO,VTP);store(PO+144,CM);store(PO+152,VP);store(PO+244,f(0.65f));store(PO+260,f(1.0f));store(PO+264,f(7.0f));
  store(PO+168,f(47.0f));store(PO+192,CH);
  if(st==19){store(CH+28,0x40013400u);store(0x40013400u+4,0x40013500u);store(0x40013500u+12,0x40013600u);}
  store(QU+0x3F0,st==19);
  store(D+12,CAR);store(D+40,FX);store(D+44,PO);store(D+48,HE);store(D+28,st);
  for(int i=0;i<4;i++)store(D+160+4*i,cut[i]);
  for(int q2=0;q2<6;q2++){for(int i=0;i<3;i++)store(D+64+16*q2+4*i,mo[3*q2+i]);store(D+64+16*q2+12,f(1.0f));}
  if(st==19){Context c=fresh();c.r3.u32=PO;sub_PACT(c,nullptr);} else if(st==18){Context c=fresh();c.r3.u32=HE;sub_HACT(c,nullptr);} else {Context c=fresh();c.r3.u32=D;sub_MNT(c,nullptr);}
  Context c=fresh();c.r3.u32=D;c.r4.u32=ah;sub_SHOT(c,nullptr);
  printf("%u %08x %08x %08x %08x\n",c.r3.u32&255,load(G),load(G+4),load(HE+168),load(HE+364));
 }
}
'''

QU = 0x40014000
ANS = 'uint32_t i=load(%d);ctx.r3.u64=i<load(%d)?load(%d+8+4*i):0;store(%d,i+1);' % (QU, QU + 4, QU, QU)
# a touch's contact (= the start s) and normal (0, 1, 0) written at the result r +16 / +32
HIT = 'if(ctx.r3.u64){for(int j=0;j<3;j++)store(R+16+4*j,load(S+4*j));store(R+32,0);store(R+36,0x3F800000u);store(R+40,0);}'
STUBS = {
    'BALL': 'printf("P%08x,%08x,%08x,%08x,%x ",load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),[&]{float x=float(ctx.f1.f64);uint32_t u;'
            'std::memcpy(&u,&x,4);return u;}(),ctx.r8.u32);' + ANS,
    'SWEEP': 'printf("W%08x,%08x,%08x,%08x,%08x,%08x,%08x,%x ",load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r5.u32),'
             'load(ctx.r5.u32+4),load(ctx.r5.u32+8),[&]{float x=float(ctx.f1.f64);uint32_t u;std::memcpy(&u,&x,4);return u;}(),ctx.r9.u32);' + ANS
             + 'if(load(0x400143F0u)){' + HIT.replace('R', 'ctx.r7.u32').replace('S', 'ctx.r4.u32') + '}',
    'HSWEEP': 'printf("H%08x,%08x,%08x,%08x,%08x,%08x,%08x,%x ",ctx.v2.u32[3],ctx.v2.u32[2],ctx.v2.u32[1],ctx.v1.u32[3],ctx.v1.u32[2],ctx.v1.u32[1],'
              '[&]{float x=float(ctx.f1.f64);uint32_t u;std::memcpy(&u,&x,4);return u;}(),ctx.r7.u32);' + ANS,
    'BOUND': 'ctx.r3.u64=0x40012C00u;',
    'QDIR': 'printf("O%08x,%08x,%08x,%08x,%08x,%08x,%08x,%u ",load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r4.u32+16),'
            'load(ctx.r4.u32+20),load(ctx.r4.u32+24),[&]{float x=float(ctx.f1.f64);uint32_t u;std::memcpy(&u,&x,4);return u;}(),ctx.r7.u32);'
            'store(0x400143E0u,ctx.r6.u32);store(0x400143E4u,ctx.r4.u32);',
    'QTEST': 'if(ctx.r4.u32!=0x40013600u||ctx.r5.u32!=0x40013400u)printf("Q? ");' + ANS
             + HIT.replace('R', 'load(0x400143E0u)').replace('S', 'load(0x400143E4u)'),
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
    vts = {c: rd.vtables_of(img, c)[0][0] for c in ('mcReplayCameraFixed', 'mcReplayCameraHeli', 'mcReplayCameraPolar')}
    src = (MAIN.replace('VTH', '0x%08Xu' % vts['mcReplayCameraHeli']).replace('VTF', '0x%08Xu' % vts['mcReplayCameraFixed'])
           .replace('VTP', '0x%08Xu' % vts['mcReplayCameraPolar']))
    for k, v in names.items():
        src = src.replace('sub_' + k + '(', v + '(')
    stubs = {names[k]: b for k, b in STUBS.items()}
    exe = rd.build(idx, [names['SHOT'], names['MNT'], names['HACT'], names['PACT']], src, a.work, 'replay_shot', image=img, stubs=stubs,
                   vtables=[(v, [10]) for v in vts.values()])
    rnd = random.Random(2044)
    lines = []
    for _ in range(5000):
        st = rnd.choice([18, 18, 19, 19, 19] + list(range(6, 18)))
        lens = rnd.choice([65.0, 60.0, 47.0, rnd.uniform(20, 90)])
        asp = rnd.choice([16 / 9, 4 / 3, rnd.uniform(1, 2.5)])
        pos = [rnd.uniform(-2000, 2000), rnd.uniform(-10, 80), rnd.uniform(-2000, 2000)]
        vel = [rnd.uniform(-40, 40), rnd.uniform(-2, 2), rnd.uniform(-40, 40)]
        top = rnd.uniform(1, 2)
        spd = rnd.choice([10.0, 22.35, 30.0, math.sqrt(sum(v * v for v in vel))])
        cut = [rnd.uniform(0, 20), rnd.uniform(0, 5), rnd.uniform(0, 3), rnd.uniform(3, 30)]
        mo = [rnd.uniform(-2, 2) for _ in range(18)]
        ht = [4.0, 20.0, 2.0, 1.0, 1.0, 20.0, 5.0, *rnd.choice([(15.0, 5.0, 5.0, 30.0), (0.3, 0.2, 0.05, 0.6)])]
        dt = rnd.choice([1 / 30, 1 / 60])
        ans = [1 if rnd.random() < 0.3 else 0 for _ in range(5)]
        lines.append(f'SHOT {st:x} {rnd.choice([0, 1])} ' + ' '.join(f'{fb(x):08x}' for x in [lens, asp, *pos, *vel, top, spd, *cut, *mo])
                     + f' {rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x} ' + ' '.join(f'{fb(x):08x}' for x in [*ht, dt])
                     + f' {len(ans)} ' + ' '.join(map(str, ans)))
    cases = a.work / 'replay_shot_cases.txt'
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
