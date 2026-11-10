"""The cameras' constructors and resets run offline: unchanged generated code of
  CTOR  the polar replay camera's constructor F-38251, the helicopter's F-75429, the replay director's F-48429 (with the three
        cameras it builds: the fixed one's constructor, the polar, the helicopter) and vehTrackCS's F-59764 (with its bases),
        each on an object whose bytes were filled with a word first (the fill 0, a pattern or a random word: a word the
        constructor does not write keeps it, so the fill tells the written words from the rest).
  DIR   the director's reset F-80435 on a director the constructor built: the fixed camera's slot 9 F-20175, the polar camera's
        activate F-82095, the helicopter's F-79727 (the shot-length draw F-66400 after them) over a random generator state, a
        random tune (the director's shot-length mean / deviation / least / most, the polar lens default, the helicopter's lens range,
        match factors, rates and zoom times) and a random car (rows, place, velocity).
  TRS   the track camera's resets: the event camera's F-88984 (the base reset F-73044 and the filtered speed) and vehTrackCS's own
        slot 9 (F-73044), on a camera object of garbage words, a random computed matrix (+336), a random gear word and speed.
  SEV   the event camera's set-event F-26269 (see the section in its block).
Lines are the case, then ' -> ', then the dump. (floats as hex bits; guest addresses never appear)
Usage: camera_ctor_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

IDS = {'POLC': 'F-38251', 'HELC': 'F-75429', 'DIRC': 'F-48429', 'DIRR': 'F-80435', 'TRKC': 'F-59764', 'RSTA': 'F-88984', 'RSTB': 'F-73044',
       'FIXR': 'F-20175', 'PLD': 'F-79400', 'POLA': 'F-82095', 'HELA': 'F-79727', 'SEVT': 'F-26269', 'PCTOR': 'F-42437'}

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t V=uint32_t(-2105147392), G=V+2176;
 const uint32_t O=0x40010000;
 auto f=[](float x){uint32_t u;std::memcpy(&u,&x,4);return u;};
 auto fill=[&](uint32_t a,uint32_t n,uint32_t w){for(uint32_t j=0;j<n;j+=4)store(a+j,w);};
 auto consts=[&](){   // (the runtime-initialised vector constants the vector routines read)
  for(int i=0;i<3;i++)store(V+2384+4*i,0xFFFFFFFFu);store(V+2384+12,0);
  for(int i=0;i<3;i++)store(V+2352+4*i,0);store(V+2352+12,f(1.0f));
  for(int i=0;i<4;i++){store(V+2240+4*i,0);store(V+2528+4*i,0x7F800000u);}};
 while(in>>k){rt_reset();consts();
  if(k=="CTOR"){uint32_t w;in>>std::hex>>w;
   const uint32_t POL=O,HEL=O+0x1000,TRK=O+0x2000,DIR=O+0x3000;
   fill(O,0x4000,w);
   {Context c=fresh();c.r3.u32=POL;sub_POLC(c,nullptr);}
   {Context c=fresh();c.r3.u32=HEL;sub_HELC(c,nullptr);}
   {Context c=fresh();c.r3.u32=TRK;sub_TRKC(c,nullptr);}
   {Context c=fresh();c.r3.u32=DIR;sub_DIRC(c,nullptr);}
   for(uint32_t o:{272,276,280,284,288})printf("%08x ",load(POL+o));printf("| ");
   for(uint32_t o:{280,284,288,320,324,328,332,340,344,348,352,356,360})printf("%08x ",load(HEL+o));printf("%u | ",load(HEL+368)>>24);
   for(uint32_t o:{12,16,20,24,28,32})printf("%08x ",load(DIR+o));printf("%u %u ",load(DIR+36)>>24,(load(DIR+36)>>16)&255);
   for(uint32_t o:{160,164,168,172})printf("%08x ",load(DIR+o));
   for(uint32_t o=64;o<160;o+=4)printf("%08x ",load(DIR+o));
   uint32_t p0=load(DIR+40),p1=load(DIR+44),p2=load(DIR+48);
   printf("%u %u %u | ",p1-p0,p2-p1,arena_top-p2);
   for(uint32_t o=4;o<0x340;o+=4)printf("%08x ",load(TRK+o));
   uint32_t lk=load(TRK+576);for(uint32_t o=0;o<48;o+=4)printf("%08x ",load(lk+o));
   printf("\n");}
  else if(k=="DIR"){uint32_t lo,ca,cut[4],pol[3],hf[3],hm[3],hr[2],hc[4],r[15];in>>std::hex>>lo>>ca;
   for(auto&x:cut)in>>x;for(auto&x:pol)in>>x;for(auto&x:hf)in>>x;for(auto&x:hm)in>>x;for(auto&x:hr)in>>x;for(auto&x:hc)in>>x;for(auto&x:r)in>>x;
   const uint32_t D=O,CM=O+0x1000,CAR=O+0x1400,TU=O+0x1800;
   fill(O,0x2000,0);
   {Context c=fresh();c.r3.u32=D;sub_DIRC(c,nullptr);}
   uint32_t pc=load(D+44),hl=load(D+48);
   for(int i=0;i<4;i++)store(D+160+4*i,cut[i]);
   store(pc+280,pol[0]);store(pc+284,pol[1]);store(pc+288,pol[2]);
   store(hl+280,hf[0]);store(hl+284,hf[1]);store(hl+288,hf[2]);store(hl+320,hm[0]);store(hl+324,hm[1]);store(hl+328,hm[2]);
   store(hl+340,hr[0]);store(hl+344,hr[1]);for(int i=0;i<4;i++)store(hl+348+4*i,hc[i]);
   store(hl+144,CM);store(hl+192,CAR);store(CAR+92,TU);store(TU+124,f(1.4f));
   store(hl+240,0);store(hl+244,f(0.65f));store(hl+248,0);store(hl+252,0);
   store(hl+256,0);store(hl+260,f(30.0f));store(hl+264,f(25.0f));store(hl+268,0);
   for(int row=0;row<4;row++){for(int i=0;i<3;i++)store(CM+16*row+4*i,r[3*row+i]);store(CM+16*row+12,row==3?f(1.0f):0);}
   for(int i=0;i<3;i++)store(CAR+400+4*i,r[12+i]);store(CAR+412,0);
   store(G,lo);store(G+4,ca);store(G+8,0);store(G+12,0);
   // (the director's state before: garbage the reset must set)
   store(D+28,0x55);store(D+32,0x12345678);store(D+36,0x00010000u);
   Context c=fresh();c.r3.u32=D;sub_DIRR(c,nullptr);
   printf("%u %08x %u %u | ",load(D+28),load(D+32),load(D+36)>>24,(load(D+36)>>16)&255);
   printf("%08x %08x %08x | ",load(pc+168),load(pc+272),load(pc+276));
   for(int row=0;row<4;row++)for(int i=0;i<3;i++)printf("%08x ",load(hl+16+16*row+4*i));
   for(int i=0;i<3;i++)printf("%08x ",load(hl+304+4*i));
   printf("%08x %08x %08x %08x %u %08x | %08x %08x %u %08x\n",load(hl+168),load(hl+336),load(hl+276),load(hl+364),load(hl+368)>>24,load(hl+332),load(G),load(G+4),load(G+8)>>24,load(G+12));}
  else if(k=="PLD"){uint32_t seed,x,y;in>>std::hex>>seed>>x>>y;
   uint32_t s=seed;uint32_t before[0x340/4];
   for(uint32_t j=0;j<0x340;j+=4){s=s*1664525u+1013904223u;store(O+j,s);}
   store(O+496,x);store(O+504,y);
   for(uint32_t j=0;j<0x340;j+=4)before[j/4]=load(O+j);
   Context c=fresh();c.r3.u32=O;sub_PLD(c,nullptr);
   printf("%08x %08x",load(O+276),load(O+272));
   for(uint32_t j=0;j<0x340;j+=4)if(load(O+j)!=before[j/4])printf(" !%u",j);
   printf("\n");}
  else if(k=="SEV"){int32_t e,cur,cnt;uint32_t en,gl,gc,apprt,rec[20],yaw,steer,pad,wz;
   in>>std::dec>>e>>cur>>std::hex>>en;for(auto&x:rec)in>>x;in>>apprt>>yaw>>steer>>pad>>std::dec>>cnt>>std::hex>>wz>>gl>>gc;
   const uint32_t C=O,TG=O+0x1000,CH=O+0x1100,PAD=O+0x1200,BODY=O+0x1300,LIST=O+0x1400,WH=O+0x1500;
   uint32_t s=0x1234567u+e*31+cur;uint32_t before[0xC00/4];
   for(uint32_t j=0;j<0xC00;j+=4){s=s*1664525u+1013904223u;store(C+j,s);}
   fill(TG,0x100,0);fill(CH,0x600,0);
   store(C+192,TG);store(TG+8,CH);store(TG+48,PAD);store16(PAD+18,pad&0xFFFF);store(CH+128,BODY);store(BODY+292,yaw);store(CH+132,steer);
   store(CH+404,LIST);store(LIST+20,cnt);store(LIST+24,WH);store(WH+392,wz);
   store(C+784,uint32_t(cur));for(int i=0;i<18;i++)pointer(C+2816+i)[0]=1;if(e>=0&&e<18)pointer(C+2816+e)[0]=en;
   if(e>=0&&e<18){uint32_t R=C+800+112*e;
    store(R+20,rec[0]);store(R+24,rec[1]);for(int i=0;i<4;i++){store(R+32+4*i,i<3?rec[2+i]:0x3F800000u);store(R+48+4*i,i<3?rec[5+i]:0x3F800000u);}
    pointer(R+64)[0]=rec[8];for(int i=0;i<11;i++)store(R+68+4*i,rec[9+i]);}
   store(C+264,apprt);
   store(G,gl);store(G+4,gc);
   for(uint32_t j=0;j<0xC00;j+=4)before[j/4]=load(C+j);
   Context c=fresh();c.r3.u32=C;c.r4.u32=uint32_t(e);sub_SEVT(c,nullptr);
   printf("%d %d %u |",int(c.r3.u32),int(load(C+784)),pointer(C+2816+(e>=0?e:0))[0]);
   const uint32_t KN[]={160,240,244,248,252,448,452,456,460,256,268,296,472,488,492,496,500,504,508,512,744,748,752,756,760};
   for(uint32_t o:KN)printf(" %08x",load(C+o));
   printf(" %u | %08x %08x |",pointer(C+572)[0],load(G),load(G+4));
   for(uint32_t j=0;j<0xC00;j+=4){bool kn=j==784||(j&~3u)==((2816+(e>=0?e:0))&~3u)||(j&~3u)==572;for(uint32_t o:KN)kn|=o==j;
    if(!kn&&load(C+j)!=before[j/4])printf(" !%u",j);}
   printf("\n");}
  else if(k=="TRS"){uint32_t seed,gear,spd,fov,cm[16];in>>std::hex>>seed>>gear>>spd>>fov;for(auto&x:cm)in>>x;
   const uint32_t C=O,TG=O+0x1000,CH=O+0x1100,TR=O+0x1200;
   for(int v=0;v<2;v++){
    uint32_t s=seed;for(uint32_t j=0;j<0x340;j+=4){s=s*1664525u+1013904223u;store(C+j,s);}
    fill(TG,0x400,0);
    store(C+192,TG);store(TG+8,CH);store(CH+20,TR);store(TR+20,gear);store(CH+224,spd);store(C+744,fov);
    for(int i=0;i<16;i++)store(C+336+4*i,cm[i]);
    uint32_t before[0x340/4];for(uint32_t j=0;j<0x340;j+=4)before[j/4]=load(C+j);
    Context c=fresh();c.r3.u32=C;if(v==0)sub_RSTA(c,nullptr);else sub_RSTB(c,nullptr);
    for(uint32_t j=16;j<80;j+=4)printf("%08x ",load(C+j));
    // (the words the resets are known to write, whatever they held: offset=value; then any other word that changed, !offset=value)
    for(uint32_t j:{156,168,220,224,228,296,436,528,532,596,656,660,664,668,672,768,772,776})printf("%u=%08x ",j,load(C+j));
    for(uint32_t j=0;j<0x340;j+=4){bool exp=false;for(uint32_t e:{156,168,220,224,228,296,436,528,532,596,656,660,664,668,672,768,772,776})exp|=e==j;
     if(!exp&&(j<16||j>=80)&&load(C+j)!=before[j/4])printf("!%u=%08x ",j,load(C+j));}
    printf(v==0?"| ":"");}
   printf("\n");}
 }
}
'''


# the player camera's constructor F-42437 (it builds the camera set: the ability camera +68 and the wipeout camera +72 are vehTrackCS
# objects, made by F-59764 and then given their own values). Its manager calls (the car's look-up and the camera registry, which
# reach the game's global state) are stubs that leave r3 alone; the car object r4 has +136 = a stand-in whose +12 is negative.
MAIN2 = r"""
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t V=uint32_t(-2105147392);
 const uint32_t O=0x40010000;
 auto f=[](float x){uint32_t u;std::memcpy(&u,&x,4);return u;};
 while(in>>k){rt_reset();
  for(int i=0;i<3;i++)store(V+2384+4*i,0xFFFFFFFFu);store(V+2384+12,0);
  for(int i=0;i<3;i++)store(V+2352+4*i,0);store(V+2352+12,f(1.0f));
  for(int i=0;i<4;i++){store(V+2240+4*i,0);store(V+2528+4*i,0x7F800000u);}
  uint32_t w;in>>std::hex>>w;
  const uint32_t P=O,CARO=O+0x1000,SUB=O+0x1200;
  for(uint32_t j=0;j<0x2000;j+=4)store(O+j,w);
  store(CARO+136,SUB);store(SUB+12,0xFFFFFFFFu);store(P+80,0);   // (+80: the selected view's index, which the stubbed registry call would store)
  Context c=fresh();c.r3.u32=P;c.r4.u32=CARO;sub_PCTOR(c,nullptr);
  for(int q=0;q<2;q++){uint32_t cam=load(P+(q==0?68:72));
   for(uint32_t o=4;o<0x2E0;o+=4)printf("%08x ",load(cam+o));printf("| ");}
  printf("\n");}
}
"""


PC_TARGETS = ['F-96575', 'F-64958', 'F-38722', 'F-62498', 'F-10995']      # (virtual calls the constructor makes: found by running it and reading the error)
# (the tune base class's slot 2, which the constructor calls on the event camera: it reaches the string and file layer; its effect on
# the camera's fields is not what is compared, so it does nothing here)
PC_FAKES = {'F-82735': ''}


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def mat(rnd):
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(x * x for x in q))
    x, y, z, w = (v / n for v in q)
    return [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)], [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
            [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]


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
    src = MAIN
    for k, v in names.items():
        src = src.replace('sub_' + k + '(', v + '(')
    # (indirect calls: the director's reset runs slot 9 of the three cameras it built)
    targets = [int(names[k][4:], 16) for k in ('FIXR', 'POLA', 'HELA')]
    exe = rd.build(idx, [names[k] for k in ('POLC', 'HELC', 'DIRC', 'DIRR', 'TRKC', 'RSTA', 'RSTB', 'PLD', 'SEVT')], src, a.work, 'camera_ctor', image=img, targets=targets)
    # (the player camera's constructor: its own build, the closure is large)
    pc_stubs = {addr(a.tool, i): '' for i in ('F-98850', 'F-82384', 'F-81018', 'F-73450', 'F-33636', 'F-35730', 'F-19507')}
    # (the reference-count release F-75034 is an atomic decrement)
    pc_stubs[addr(a.tool, 'F-75034')] = 'uint32_t v=load(ctx.r3.u32)-1;store(ctx.r3.u32,v);ctx.r3.u32=v;'
    pc_fakes = {int(addr(a.tool, i)[4:], 16): b for i, b in PC_FAKES.items()}
    pc_src = MAIN2.replace('sub_PCTOR(', names['PCTOR'] + '(')
    rnd = random.Random(20261109)
    lines = []
    for w in [0, 0xA5A5A5A5, 0x3F800000, 0x7FC00000, 0xFFFFFFFF] + [rnd.getrandbits(32) & 0x7FFFFFFF | 0x01000000 for _ in range(30)]:
        lines.append(f'CTOR {w:08x}')
    for _ in range(3000):
        m = mat(rnd)
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 100), rnd.uniform(-3000, 3000)]
        if rnd.random() < 0.2:
            m = [m[0], [0.0, 1.0, 0.0], [rnd.choice([0.0, 1.0, -1.0]), 0.0, rnd.choice([1.0, -1.0, 0.0])]]
        vel = [rnd.uniform(-60, 60), rnd.uniform(-10, 10), rnd.uniform(-60, 60)] if rnd.random() < 0.85 else [0.0, 0.0, 0.0]
        cut = rnd.choice([(5.0, 3.0, 1.0, 10.0), (rnd.uniform(0, 20), rnd.uniform(0, 8), rnd.uniform(0, 5), rnd.uniform(5, 40)),
                          (rnd.uniform(0, 20), rnd.uniform(0, 8), rnd.uniform(5, 40), rnd.uniform(0, 5))])
        pol = rnd.choice([(25.0, 60.0, 47.0), (rnd.uniform(5, 40), rnd.uniform(40, 90), rnd.uniform(10, 90))])
        hlo = rnd.uniform(1, 10)
        hf = rnd.choice([(4.0, 20.0, 7.0), (hlo, hlo + rnd.uniform(0, 30), rnd.uniform(1, 40))])
        hm = rnd.choice([(2.0, 1.0, 1.0), (rnd.uniform(0, 4), rnd.uniform(0, 3), rnd.uniform(0, 3))])
        hr = rnd.choice([(20.0, 5.0), (rnd.uniform(1, 40), rnd.uniform(0, 10))])
        hc = rnd.choice([(15.0, 5.0, 5.0, 30.0), (rnd.uniform(0, 20), rnd.uniform(0, 8), rnd.uniform(0, 5), rnd.uniform(5, 40))])
        vals = list(cut) + list(pol) + list(hf) + list(hm) + list(hr) + list(hc) + [c for r in m for c in r] + place + vel
        lines.append(f'DIR {rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x} ' + ' '.join(f'{fb(v):08x}' for v in vals))
    for _ in range(3000):
        gear = rnd.choice([0, 0, 1, 2, 3, 6, 0xFFFFFFFF, rnd.getrandbits(32)])
        spd = rnd.choice([0.0, -0.0, rnd.uniform(0, 80), rnd.uniform(-5, 5), 1e-30, 1e30, float('inf'), float('nan')])
        fov = rnd.choice([40.0, 60.0, rnd.uniform(20, 90)])
        cm = [rnd.choice([rnd.uniform(-3000, 3000), rnd.gauss(0, 1), 0.0, 1.0]) for _ in range(16)]
        lines.append(f'TRS {rnd.getrandbits(32):08x} {gear:08x} {fb(spd):08x} {fb(fov):08x} ' + ' '.join(f'{fb(v):08x}' for v in cm))
    for _ in range(1000):
        x = rnd.choice([rnd.uniform(0, 30), 0.0, -0.0, 5.0, 12.0, float('inf'), float('nan'), rnd.uniform(-5, 5)])
        y = rnd.choice([rnd.uniform(0, 30), 0.0, 5.0, rnd.uniform(-5, 5)])
        lines.append(f'PLD {rnd.getrandbits(32):08x} {fb(x):08x} {fb(y):08x}')
    for _ in range(4000):
        e = rnd.choice([rnd.randrange(18)] * 6 + [-1, rnd.randrange(18), 7, 7, 13, 5, 11])
        cur = rnd.choice([-1, rnd.randrange(-1, 18), rnd.randrange(-1, 18), e if rnd.random() < 0.08 else -1])
        en = rnd.choice([1, 1, 1, 1, 0, 1])
        rec = [rnd.uniform(0, 3), rnd.choice([0.0, rnd.uniform(0, 3)]),
               rnd.choice([rnd.uniform(-2, 2), 0.0]), rnd.uniform(-2, 3), rnd.uniform(-4, 4),
               rnd.choice([rnd.uniform(-3, 3), 0.0]), rnd.uniform(-2, 3), rnd.uniform(-10, 10)]
        rec = [fb(v) for v in rec] + [rnd.choice([0, 1, 1, 2, 255])] + [fb(rnd.choice([rnd.uniform(0, 40), 0.0]))] + [fb(rnd.uniform(0, 20)) for _ in range(10)]
        sides = [0.0, -0.0, float('nan'), rnd.uniform(-3, 3), rnd.uniform(-0.01, 0.01), 1e-30, -1e-30]
        lines.append('SEV %d %d %x ' % (e, cur, en) + ' '.join(f'{v:08x}' for v in rec) + f' {fb(rnd.choice([60.0, rnd.uniform(1, 90)])):08x} '
                     f'{fb(rnd.choice(sides)):08x} {fb(rnd.choice(sides)):08x} {rnd.choice([0, 1, 5, 0x7FFF, 0x8000, 0xFFFF, rnd.getrandbits(16)]):x} '
                     f'{rnd.choice([0, 1, 2, 4]):d} {fb(rnd.uniform(-3, 3)):08x} {rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x}')
    pc_lines = [f'PC {w:08x}' for w in (0, 0xA5A5A5A5, 0x3F800000, 0x01234567)]
    cases = a.work / 'camera_ctor_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines), (len(rows), len(lines))
    out = [l + ' -> ' + r for l, r in zip(lines, rows)]
    pcases = a.work / 'camera_pc_cases.txt'
    pcases.write_text(chr(10).join(pc_lines) + chr(10))
    pc_targets = [int(addr(a.tool, i)[4:], 16) for i in PC_TARGETS]
    while True:
        pc_exe = rd.build(idx, [names['PCTOR']], pc_src, a.work, 'camera_pc', image=img, stubs=pc_stubs, targets=pc_targets, fakes=pc_fakes, limit=800)
        p = subprocess.run([str(pc_exe), str(a.image), str(pcases)], capture_output=True, text=True)
        m = re.search(r'unresolved indirect call 0x([0-9A-F]{8})', p.stderr)
        if not m:
            break
        pc_targets.append(int(m.group(1), 16))      # (a virtual call the constructor makes on an object it built: its code joins the build)
        print('indirect target added: F-' + subprocess.run([a.tool, 'registry', 'id', m.group(1)], capture_output=True, text=True).stdout.split()[0][2:])
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    prow = p.stdout.splitlines()
    assert len(prow) == len(pc_lines), (len(prow), len(pc_lines), p.stderr[-300:])
    out += [l + ' -> ' + r for l, r in zip(pc_lines, prow)]
    bad = [l for l in out if re.search(r'(?<![0-9a-zA-Z])82[0-9a-f]{6}(?![0-9a-zA-Z])', l)]
    keep = [l for l in out if l not in bad]
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(keep), 'of', len(out), 'cases')


if __name__ == '__main__':
    main()
