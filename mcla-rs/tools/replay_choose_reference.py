"""The replay director's frame update (F-19930) and choose (F-53721) run offline: unchanged generated code with the real activate
(F-82762), the mount setter, the shot length, the normal deviate and the k-th bit; the cameras' activates (slot 9: the fixed camera's
reset, the polar camera's activate F-82095, the helicopter's F-79727) go through their real vtables (found by class name).
Stubbed: the shot test F-51014 (it answers from the case's list and logs the state and whether it looks ahead: the test itself is
checked apart), the car's draw flags F-28331 (logged), the director's cut F-53309 (sets the director's current camera: what choose
compares afterwards), the dynamic cast F-75805 (0, so the boom snap F-17419 is not reached), the transition F-28091 (logged with its
three floats), the C library's memset F-29294 (as a byte loop). The five drive views and the reverse camera are host objects whose slot 9 does nothing and slot 15 returns their
fixed-mount byte +668 (1 for views 0 and 1); the director's slot 9 is logged.
Object (the replay camera): +12 -> car, +16 -> director (+48 current camera), +20 reverse camera, +24 -> the drive views' table, +28 state,
+32 shot time, +36 switching, +37 next, +40 -> fixed, +44 -> polar, +48 -> helicopter, +64 + 16 k mounts, +160 .. +172 shot length
(mean, deviation, least, most). The helicopter object as in replay_heli_reference.py, the polar one with its default lens +288.
Set-up as the port's Director::new: polar activate, helicopter activate, shot time = F-66204; then the state.
Line:  CHO <lo> <carry> <state> <cut x4> <heli: lens lo hi, k0 k1 k2, lens rate, turn rate, cut x4> <polar default lens> <car place x3>
       <car velocity x3> <mounts x18> <n answers> <answers (0 / 1)> <n frames> <frames: dt switching next>
       -> per frame: [events, each followed by a space: S<state>,<ahead> D<a>,<b>,<c>,<d> T<f1>,<f2>,<f3> R] <state> <time> <next> <lo> <carry> <spare flag> <spare> <heli lens> <heli angle> <heli side> <heli zoom>
          <heli place x3> <polar lens> <polar pitch> <polar yaw>
       (frames separated by ' | ', floats as hex bits)
Usage: replay_choose_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = {'UPD': 'F-19930', 'SHOT': 'F-51014', 'DRAW': 'F-28331', 'CUT': 'F-53309', 'CAST': 'F-75805', 'SNAP': 'F-17419',
       'TRANS': 'F-28091', 'FILL': 'F-29294', 'PACT': 'F-82095', 'HACT': 'F-79727', 'LEN': 'F-66204'}

MAIN = r'''
static uint32_t gViews[6];
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t G=uint32_t(-2105147392)+2176, GDT=uint32_t(-2105737216)+29952+8, V=uint32_t(-2105147392);
 const uint32_t D=0x40010000,M=0x40010400,CM=0x40010800,CAR=0x40010C00,TU=0x40011000,FX=0x40011400,PO=0x40011800,HE=0x40011C00,
  VW=0x40012000,VT=0x40013000,MVT=0x40013100,TB=0x40013200,QU=0x40014000;
 auto f=[](float x){uint32_t u;std::memcpy(&u,&x,4);return u;};
 while(in>>k){rt_reset();
  for(int i=0;i<3;i++)store(V+2384+4*i,0xFFFFFFFFu);store(V+2384+12,0);
  for(int i=0;i<3;i++)store(V+2352+4*i,0);store(V+2352+12,f(1.0f));
  for(int i=0;i<4;i++){store(V+2240+4*i,0);store(V+2528+4*i,0x7F800000u);}
  for(uint32_t a=0x40010000;a<0x40013400;a+=4)store(a,0);
  uint32_t lo,ca,st,cut[4],ht[11],pdef,pos[3],vel[3],mo[18];int n,nf;
  in>>std::hex>>lo>>ca>>st;for(auto&x:cut)in>>x;for(auto&x:ht)in>>x;in>>pdef;for(auto&x:pos)in>>x;for(auto&x:vel)in>>x;for(auto&x:mo)in>>x;
  in>>std::dec>>n;store(QU,0);store(QU+4,n);for(int i=0;i<n;i++){int b;in>>b;store(QU+8+4*i,b);}
  store(G,lo);store(G+4,ca);store(G+8,0);store(G+12,0);
  // the car (identity rows) and the helicopter / polar objects
  for(int r=0;r<3;r++)store(CM+16*r+4*r,f(1.0f));for(int i=0;i<3;i++)store(CM+48+4*i,pos[i]);store(CM+60,f(1.0f));
  for(int i=0;i<3;i++)store(CAR+400+4*i,vel[i]);store(CAR+92,TU);store(TU+124,f(1.4f));
  store(HE,VTH);store(HE+144,CM);store(HE+192,CAR);store(HE+244,f(0.65f));store(HE+260,f(30.0f));store(HE+264,f(25.0f));
  store(HE+280,ht[0]);store(HE+284,ht[1]);store(HE+288,f(7.0f));store(HE+320,ht[2]);store(HE+324,ht[3]);store(HE+328,ht[4]);
  store(HE+340,ht[5]);store(HE+344,ht[6]);for(int i=0;i<4;i++)store(HE+348+4*i,ht[7+i]);
  store(PO,VTP);store(PO+144,CM);store(PO+192,CAR);store(PO+244,f(0.65f));store(PO+260,f(1.0f));store(PO+264,f(7.0f));store(PO+288,pdef);
  store(FX,VTF);store(FX+144,CM);
  // the drive views and the reverse camera: host vtable (slot 9 nothing, slot 15 the byte +668)
  store(VT+36,0x7F000010u);store(VT+60,0x7F000011u);
  for(int v=0;v<6;v++){gViews[v]=VW+0x300*v;store(gViews[v],VT);*pointer(gViews[v]+668)=v<2?1:0;}
  for(int v=0;v<5;v++)store(TB+4*v,gViews[v]);
  store(MVT+36,0x7F000012u);store(M,MVT);
  store(D+12,CAR);store(D+16,M);store(D+20,gViews[5]);store(D+24,TB);store(D+40,FX);store(D+44,PO);store(D+48,HE);
  for(int i=0;i<4;i++)store(D+160+4*i,cut[i]);
  for(int q2=0;q2<6;q2++){for(int i=0;i<3;i++)store(D+64+16*q2+4*i,mo[3*q2+i]);store(D+64+16*q2+12,f(1.0f));}
  {Context c=fresh();c.r3.u32=PO;sub_PACT(c,nullptr);}
  {Context c=fresh();c.r3.u32=HE;sub_HACT(c,nullptr);}
  {Context c=fresh();c.r3.u32=D;sub_LEN(c,nullptr);}
  store(D+28,st);*pointer(D+36)=1;store(M+48,st<5?gViews[st]:st==5?gViews[5]:st==18?HE:st==19?PO:FX);
  in>>nf;
  for(int s=0;s<nf;s++){uint32_t dt,sw,nx;in>>std::hex>>dt>>sw>>nx>>std::dec;store(GDT,dt);*pointer(D+36)=sw;if(nx)*pointer(D+37)=1;
   Context c=fresh();c.r3.u32=D;sub_UPD(c,nullptr);
   printf("%x %08x %u %08x %08x %u %08x ",load(D+28),load(D+32),load(D+36)>>16&255,load(G),load(G+4),load(G+8)>>24,load(G+12));
   printf("%08x %08x %u %08x %08x %08x %08x ",load(HE+168),load(HE+276),load(HE+368)>>24,load(HE+364),load(HE+64),load(HE+68),load(HE+72));
   printf("%08x %08x %08x",load(PO+168),load(PO+272),load(PO+276));printf(s+1<nf?" | ":"\n");}
 }
}
'''

QU = 0x40014000      # the answers: +0 next, +4 count, +8 the list
STUBS = {
    'SHOT': 'printf("S%%u,%%u ",load(ctx.r3.u32+28),ctx.r4.u32&255);uint32_t i=load(%d);'
            'ctx.r3.u64=i<load(%d)?load(%d+8+4*i):1;store(%d,i+1);' % (QU, QU + 4, QU, QU),
    'DRAW': 'printf("D%u,%u,%u,%u ",ctx.r4.u32&255,ctx.r5.u32&255,ctx.r6.u32&255,ctx.r7.u32&255);',
    'CUT': 'store(ctx.r3.u32+48,ctx.r4.u32);',
    'CAST': 'ctx.r3.u64=0;',
    'SNAP': '',
    'FILL': 'for(uint32_t i=0;i<ctx.r5.u32;i++)*pointer(ctx.r3.u32+i)=uint8_t(ctx.r4.u32);',     # the C library's memset
    'TRANS': 'float a=float(ctx.f1.f64),b=float(ctx.f2.f64),c=float(ctx.f3.f64);uint32_t x,y,z;std::memcpy(&x,&a,4);std::memcpy(&y,&b,4);'
             'std::memcpy(&z,&c,4);printf("T%08x,%08x,%08x ",x,y,z);',
}
FAKES = {0x7F000010: '', 0x7F000011: 'ctx.r3.u64=*pointer(ctx.r3.u32+668);', 0x7F000012: 'printf("R ");'}


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
    vts = {c: rd.vtables_of(img, c)[0][0] for c in ('mcReplayCameraFixed', 'mcReplayCameraPolar', 'mcReplayCameraHeli')}
    src = MAIN.replace('VTH', '0x%08Xu' % vts['mcReplayCameraHeli']).replace('VTP', '0x%08Xu' % vts['mcReplayCameraPolar']) \
              .replace('VTF', '0x%08Xu' % vts['mcReplayCameraFixed'])
    for k, v in names.items():
        src = src.replace('sub_' + k + '(', v + '(')
    stubs = {names[k]: b for k, b in STUBS.items()}
    exe = rd.build(idx, [names['UPD'], names['PACT'], names['HACT'], names['LEN']], src, a.work, 'replay_choose', image=img,
                   stubs=stubs, fakes=FAKES, vtables=[(v, [9, 15]) for v in vts.values()])
    rnd = random.Random(2043)
    lines = []
    for _ in range(400):
        cut = rnd.choice([(15.0, 5.0, 5.0, 30.0), (0.5, 0.3, 0.1, 1.0), (rnd.uniform(0, 3), rnd.uniform(0, 2), rnd.uniform(0, 0.5), rnd.uniform(0.5, 4))])
        lo_f, hi_f = rnd.choice([(4.0, 20.0), (rnd.uniform(2, 15), rnd.uniform(15, 60))])
        ht = [lo_f, hi_f, 2.0, 1.0, 1.0, 20.0, 5.0, *rnd.choice([(15.0, 5.0, 5.0, 30.0), (0.3, 0.2, 0.05, 0.6)])]
        pdef = rnd.choice([47.0, rnd.uniform(20, 80)])
        pos = [rnd.uniform(-2000, 2000), rnd.uniform(-10, 80), rnd.uniform(-2000, 2000)]
        vel = [rnd.uniform(-40, 40), rnd.uniform(-2, 2), rnd.uniform(-40, 40)]
        mo = [rnd.uniform(-2, 2) for _ in range(18)]
        st = rnd.randrange(20)
        p_yes = rnd.choice([0.0, 0.3, 0.7, 1.0])
        ans = [1 if rnd.random() < p_yes else 0 for _ in range(rnd.randrange(0, 120))]
        frames = []
        dt = rnd.choice([1 / 30, 1 / 60, 0.25])
        for _ in range(40):
            d = rnd.choice([dt, dt, dt, 0.0, rnd.uniform(0, 1)])
            frames.append(f'{fb(d):08x} {rnd.choice([1, 1, 1, 0])} {1 if rnd.random() < 0.08 else 0}')
        w = [lo_f, hi_f]
        lines.append(f'CHO {rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x} {st:x} ' + ' '.join(f'{fb(x):08x}' for x in [*cut, *ht, pdef, *pos, *vel, *mo])
                     + f' {len(ans)} ' + ' '.join(map(str, ans)) + f' {len(frames)} ' + ' '.join(frames))
    cases = a.work / 'replay_choose_cases.txt'
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
