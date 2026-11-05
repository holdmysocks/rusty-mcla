"""Run the original's arrest timers offline (static recompilation, native, flat guest memory) and record them for the port's
test (crates/game/src/testdata/arrest_ref.txt; test arrest_timers_match_the_original).

The Pursuit tactic object is built as the game builds it (ctor F-55508, reset F-20606, begin F-89627; as
pursuit_fsm_reference.py), with 1 to 3 cops, each with a brain, and the target (a player). Then the tactic update F-68032 runs
with a given dt, N times, with the original's code unchanged for: the update's gate and its arrest test F-36806 (states 1, 2, 3,
5, 6 only, nothing asked, the target a racer, not lost F-77220) and its ask of the Arrest state; the FSM's table and its state
changes (F-28114 ask, F-95322 update) including the Delay's exit F-25831; the common tactic update F-63286 with the seen update
F-74492 (vt+16), the arrest timer F-29906 (vt+68, behind vt+64 F-20593), the cop query F-29443 with the strict squared distance of
F-39842, the burnout F-25339 and damaged-out F-12032 queries, the +140 / +144 timers and the target-place refresh +48 (F-14038,
F-41260), vt+32 F-54202 (the radius), vt+40 F-96991, vt+44 F-77220 (the Search state's vt+80 F-73289), the first seeing cop
F-43276, the target's matrix getter F-35658.

What is not the original's code (inputs or no-ops):
 - the seven states' tick and enter are no-ops (the cases ask the next state themselves, as the state ticks would), the exits
   too except the Delay's exit F-25831; the Search state's +16 (the search clock, read by vt+80) is an input;
 - the cop sight test F-94676 (called by F-43276 with the radius + 1 or fBikerWipeoutArrestDist) is an input bit per cop;
 - the seen test F-16046 (called by F-74492) is an input bit; the bike test F-17373 is an input;
 - the tactic's vt+72 / +76 / +80 / +84 (cop formation, goals) are no-ops (vt+72 false), the cop loop's per-cop update
   F-78189, its second query F-33706 (false) and the +88 refresh F-39048 are no-ops; the proximity test F-29545 (the
   Pursuit's own Halt ask) is false; the cop is valid (F-14415).
The tune gets the values the cases vary (fSafeDist, fArrestTime, fArrestTgtThresholdSpd, fBikerWipeoutArrestDist).

Inputs come from an integer xorshift (the test makes the same ones); `gen_case` below is the spec, the Rust test mirrors it:
 the state asks (valid per the table), the Search clock, the seen bit, the target's place in 1/16 m (stop / go steps near
 the barely-moving threshold), its speed in 1/16 m/s, burnout charge, damage, bike down; per cop the offset from the target in
 1/16 m (a random walk with now and then an exact snap to the radius, radius + 1/16 m, ...), underground, damaged out, brain,
 and the two sight bits.

  CASE <seed>
  <k> <pre> <post> <fire> <+108 bits> <+140 bits> <+144 bits> <+124 bits> <+132 bits>
  (pre / post: the FSM state before / after the update; fire: the update asked Arrest; lines where something changed, every
  25th, the last and the firing one; a case ends at its firing update)
"""
import argparse
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp
import pursuit_fsm_reference as P

CASES = 40
N = 1200
MAIN_IDS = ['F-55508', 'F-20606', 'F-89627', 'F-68032']
OWN_STUBS = ['F-94676', 'F-16046', 'F-17373', 'F-29545', 'F-14415', 'F-78189', 'F-33706', 'F-12823', 'F-39048']
TARGETS = ['F-87882', 'F-54202', 'F-77220', 'F-49619', 'F-47121', 'F-74264', 'F-74492', 'F-24596', 'F-37056', 'F-82134', 'F-29906',
           'F-33778', 'F-94002', 'F-20593', 'F-96991', 'F-73289', 'F-25831']
IMG = 0x82 << 24     # (the image base; data and function places below are offsets, no raw addresses)
RAW_TARGETS = [IMG + o for o in (0x712610, 0x130008, 0x1A5CC0, 0x719A28)]
TACTIC_VT = IMG + 0x0C8704
# 1/60, 1/30, 1/30 x 0.25, 1/30 x 0.37 as f32 bits (the Rust test has the same constants)
DT_BITS = [0x3C888889, 0x3D088889, 0x3C088889, 0x3C4A11C1]
SAFE = [30, 15, 30, 45]
ARREST_T = [0.5, 1.0, 1.0, 2.5]
THR = [5, 10, 10, 20]
BIKER = [50, 50, 20, 70]
ALLOWED = {0: [1], 1: [2, 5, 6], 2: [1, 3, 5, 6], 3: [1, 6], 5: [1, 6], 6: [1]}
DWELL = {0: 40, 1: 120, 2: 90, 3: 70, 5: 90, 6: 150}


def gen_case(seed):
    """-> (params, ticks). params: dt bits, safe, arrest_t, thr, biker, ncops, bike. tick: ask, lost, seen, tx, ty, tz, spd16,
    burn code, wreck code, bikedown, then per cop ox oy oz ug wr br s1 sb"""
    g = P.Rng(seed)
    dtb = DT_BITS[g.r(0, 3)]
    safe, at, thr, biker = SAFE[g.r(0, 3)], ARREST_T[g.r(0, 3)], THR[g.r(0, 3)], BIKER[g.r(0, 3)]
    n = g.r(1, 3)
    bike = 1 if g.next() % 3 == 0 else 0
    smax = g.r(2, 5)
    t = [g.r(-300, 300) * 8, g.r(0, 20) * 8, g.r(-300, 300) * 8]
    if seed % 5 == 0:     # (the target starts at the origin's place: the reset's +48 = 0 decides the first +144)
        t = [v % 5 - 2 for v in t]
    spd = g.r(0, 120)
    go = g.r(0, 1)
    cur, lostb, seen = 0, 0, g.r(0, 1)
    burn, wreck, bdown = 0, 0, 0
    lim = (safe + 20) * 16
    cops = [dict(o=[g.r(-20, 20) * 16 for _ in range(3)], ug=0, wr=0, br=1, s1=g.r(0, 1), sb=g.r(0, 1)) for _ in range(n)]
    ticks = []
    for _ in range(N):
        ask = -1
        if g.next() % DWELL[cur] == 0:
            opts = ALLOWED[cur]
            ask = opts[g.next() % len(opts)]
        post = ask if ask != -1 else cur
        if post == 6 and ask == -1:
            if g.next() % 25 == 0:
                lostb ^= 1
        else:
            lostb = 0
        if g.next() % 12 == 0:
            seen ^= 1
        if g.next() % 40 == 0:
            go ^= 1
        if go:
            t = [t[0] + g.r(-smax, smax), t[1] + g.r(-smax, smax), t[2] + g.r(-smax, smax)]
        spd = max(0, min(280, spd + g.r(-3, 3)))
        if g.next() % 30 == 0:
            burn = g.r(0, 2)
        if g.next() % 500 == 0:
            wreck = g.r(0, 2)
        if g.next() % 60 == 0:
            bdown ^= 1
        row = [ask, lostb, seen, t[0], t[1], t[2], spd, burn, wreck, bdown]
        for ci, c in enumerate(cops):
            c['o'] = [max(-lim, min(lim, c['o'][i] + g.r(-8, 8))) for i in range(3)]
            if g.next() % 90 == 0:
                rr = [safe * 16, safe * 16 + 1, safe * 16 - 1, (safe + 1) * 16][g.r(0, 3)]
                ax = g.r(0, 2)
                sg = 1 if g.next() % 2 == 0 else -1
                c['o'] = [rr * sg if i == ax else 0 for i in range(3)]
            if g.next() % 200 == 0:
                c['ug'] ^= 1
            if g.next() % 300 == 0:
                c['wr'] = g.r(0, 2)
            if g.next() % 500 == 0 and ci > 0:     # (F-68032 reads the first cop's brain unchecked: it always has one)
                c['br'] ^= 1
            if g.next() % 15 == 0:
                c['s1'] ^= 1
            if g.next() % 15 == 0:
                c['sb'] ^= 1
            row += c['o'] + [c['ug'], c['wr'], c['br'], c['s1'], c['sb']]
        ticks.append(row)
        cur = post
    return (dtb, safe, at, thr, biker, n, bike), ticks


MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static float lf(uint32_t a){uint32_t u=load(a);float f;memcpy(&f,&u,4);return f;}
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
static uint32_t bits(float f){uint32_t u;memcpy(&u,&f,4);return u;}
static const uint32_t M=0x40800000u, TGT=0x40A10000u, G=0x40A50000u, IN=0x40D00000u, VTM=0x40C30000u, VTS=0x40C40000u;
// object blocks: car at base, physics base+0x1000 (+8 chassis, +20 damage obj, +28 matrix block), chassis base+0x2000,
// damage obj base+0x3000 (+8 -> base+0x4000: +24 max damage), matrix block base+0x5000 (origin at +64), brain base+0x8000
static uint32_t CAR(int i){return i<0?TGT:0x40B00000u+(uint32_t)i*0x10000u;}
static void mkcar(uint32_t c, uint32_t vt, bool brain, uint32_t tgt){
 store(c,vt); store(c+48,c+0x1000); store(c+0x1000+8,c+0x2000); store(c+0x1000+20,c+0x3000); store(c+0x1000+28,c+0x5000);
 store(c+0x3000+8,c+0x4000); sf(c+0x4000+24,100.0f); store(c+656,3u);
 if(brain){ uint32_t B=c+0x8000; store(c+876,B); store(B+4,c); store(B+680,tgt); sb(B+773,0); sf(B+596,0.0f);}
}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t AI=strtoul(argv[3],0,16), PL=strtoul(argv[4],0,16); std::string tok;
 while(in>>tok){ if(tok!="case") break; int N,nc,bike; uint32_t dtb; float safe,at,thr,biker; in>>N>>std::hex>>dtb>>std::dec>>safe>>at>>thr>>biker>>nc>>bike;
  rt_reset();
  sf(TUNE+192,safe); sf(TUNE+212,1.5f); sf(TUNE+304,5.0f); sf(TUNE+360,at); sf(TUNE+364,1.75f); sf(TUNE+368,thr); sf(TUNE+372,biker);
  store(MGRG,0x40C20000u);
  Context c=fresh(); c.r3.u32=M; CTOR(c,nullptr);
  c=fresh(); c.r3.u32=M; RESET(c,nullptr);
  mkcar(TGT,PL,false,0); store(TGT+656,3u);
  uint32_t arr=0x40C10000u; store(M+72,(uint32_t)nc); store(M+76,arr);
  for(int i=0;i<nc;i++){ mkcar(CAR(i),AI,true,TGT); store(arr+4*i,CAR(i)); store(CAR(i)+1076,1u); }
  store(M+32,TGT); store(M+36,G); store(M+20,(load(M+20)&0x00ffffffu)|0x01000000u); sf(M+164,100.0f);
  c=fresh(); c.r3.u32=M; BEGIN(c,nullptr);
  uint32_t F=load(M+160);
  for(int i=0;i<24;i++) store(VTM+4*i,load(TACTIC_VT+4*i));
  store(VTM+72,0x7F000010u); store(VTM+76,0x7F000011u); store(VTM+80,0x7F000010u); store(VTM+84,0x7F000011u); store(M,VTM);
  uint32_t st[7];
  for(int i=0;i<7;i++){ st[i]=load(load(F+4)+4*i); uint32_t vt=load(st[i]), cp=VTS+0x100u*i; for(int k=0;k<24;k++) store(cp+4*k,load(vt+4*k));
   store(cp+60,0x7F000011u); store(cp+68,0x7F000011u); if(i!=0) store(cp+72,0x7F000011u); store(st[i],cp); }
  printf("init %d %d\n",(int)load(F+12),(int)load(F+16));
  for(int k=0;k<N;k++){ int ask,lost,seen,tx,ty,tz,spd,burn,wreck,bd; in>>ask>>lost>>seen>>tx>>ty>>tz>>spd>>burn>>wreck>>bd;
   sb(IN,seen); store(IN+12,bike); sf(IN+16,biker);
   sf(CAR(-1)+0x5000+64,tx/16.0f); sf(CAR(-1)+0x5000+68,ty/16.0f); sf(CAR(-1)+0x5000+72,tz/16.0f); sf(CAR(-1)+0x5000+76,0.0f);
   uint32_t CH=TGT+0x2000; sf(CH+224,spd/16.0f); sf(CH+580,burn==0?0.0f:burn==1?0.5f:-0.25f); store(CH+592,(bike&&bd)?0x10u:0u);
   sf(TGT+0x3000+36,wreck==0?10.0f:wreck==1?100.0f:150.0f);
   uint32_t m1=0,mb=0;
   for(int i=0;i<nc;i++){ int ox,oy,oz,ug,wr,br,s1,sb2; in>>ox>>oy>>oz>>ug>>wr>>br>>s1>>sb2; uint32_t cc=CAR(i);
    sf(cc+0x5000+64,(tx+ox)/16.0f); sf(cc+0x5000+68,(ty+oy)/16.0f); sf(cc+0x5000+72,(tz+oz)/16.0f); sf(cc+0x5000+76,0.0f);
    sb(cc+880,ug); sf(cc+0x3000+36,wr==0?10.0f:wr==1?100.0f:150.0f); store(cc+876,br?cc+0x8000:0u); m1|=(uint32_t)s1<<i; mb|=(uint32_t)sb2<<i; }
   store(IN+4,m1); store(IN+8,mb);
   sf(st[6]+16,lost?0.37f:0.0f);
   uint32_t pre=load(F+12);
   if(ask>=0){ c=fresh(); c.r3.u32=F; c.r4.u32=(uint32_t)ask; ASK(c,nullptr); }
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)lf2(dtb); UPDATE(c,nullptr);
   uint32_t post=load(F+12), pend=load(F+16); int fire=(pend==4||post==4);
   printf("%d %d %d %d %08x %08x %08x %08x %08x\n",k,(int)pre,(int)post,fire,bits(lf(M+108)),bits(lf(M+140)),bits(lf(M+144)),bits(lf(M+124)),bits(lf(M+132)));
   if(fire){ for(int j=k;j<N;j++){ std::string skip; std::getline(in,skip);} break; } }
  printf("end\n");}
}
'''


MAIN_VT = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static float lf(uint32_t a){uint32_t u=load(a);float f;memcpy(&f,&u,4);return f;}
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
static uint32_t bits(float f){uint32_t u;memcpy(&u,&f,4);return u;}
static const uint32_t M=0x40800000u, TGT=0x40A10000u, G=0x40A50000u, IN=0x40D00000u, VTM=0x40C30000u, VTS=0x40C40000u;
static uint32_t CAR(int i){return i<0?TGT:0x40B00000u+(uint32_t)i*0x10000u;}
static void mkcar(uint32_t c, uint32_t vt, bool brain, uint32_t tgt){
 store(c,vt); store(c+48,c+0x1000); store(c+0x1000+8,c+0x2000); store(c+0x1000+20,c+0x3000); store(c+0x1000+28,c+0x5000);
 store(c+0x3000+8,c+0x4000); sf(c+0x4000+24,100.0f); store(c+656,3u);
 if(brain){ uint32_t B=c+0x8000; store(c+876,B); store(B+4,c); store(B+680,tgt); sb(B+773,0); sf(B+596,0.0f);}
}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t AI=strtoul(argv[3],0,16), PL=strtoul(argv[4],0,16); std::string tok;
 while(in>>tok){ if(tok!="case") break; int N,nc,bike,cls; uint32_t dtb; float safe,at,thr,biker; in>>N>>std::hex>>dtb>>std::dec>>safe>>at>>thr>>biker>>nc>>bike>>cls;
  rt_reset();
  sf(TUNE+192,safe); sf(TUNE+196,safe); sf(TUNE+212,1.5f); sf(TUNE+304,5.0f); sf(TUNE+360,at); sf(TUNE+364,1.75f); sf(TUNE+368,thr); sf(TUNE+372,biker);
  store(MGRG,0x40C20000u);
  Context c=fresh(); c.r3.u32=M; CTOR(c,nullptr);
  c=fresh(); c.r3.u32=M; RESET(c,nullptr);
  mkcar(TGT,PL,false,0); store(TGT+656,3u);
  uint32_t arr=0x40C10000u; store(M+72,(uint32_t)nc); store(M+76,arr);
  for(int i=0;i<nc;i++){ mkcar(CAR(i),AI,true,TGT); store(arr+4*i,CAR(i)); store(CAR(i)+1076,1u); }
  store(M+32,TGT); store(M+36,G); store(M+20,(load(M+20)&0x00ffffffu)|0x01000000u); sf(M+164,100.0f);
  c=fresh(); c.r3.u32=M; BEGIN(c,nullptr);
  uint32_t F=load(M+160);
  // the class of the case: the pull-over's or the arrow trap's vtable (slots 72 / 76 / 80 / 84 no-ops), over the same object
  uint32_t CVT = cls==0 ? CVT0 : CVT1;
  for(int i=0;i<24;i++) store(VTM+4*i,load(CVT+4*i));
  store(VTM+72,0x7F000010u); store(VTM+76,0x7F000011u); store(VTM+80,0x7F000010u); store(VTM+84,0x7F000011u); store(M,VTM);
  uint32_t st[7];
  for(int i=0;i<7;i++) st[i]=load(load(F+4)+4*i);
  store(load(F+4)+12,st[6]);   // the arrow trap's Search is state 3: the Search state object (vt+80 F-73289) stands there
  printf("init %d\n",(int)load(F+12));
  int cur=0;
  for(int k=0;k<N;k++){ int ask,lost,seen,tx,ty,tz,spd,burn,wreck,bd; in>>ask>>lost>>seen>>tx>>ty>>tz>>spd>>burn>>wreck>>bd;
   sb(IN,seen); store(IN+12,bike); sf(IN+16,biker);
   sf(CAR(-1)+0x5000+64,tx/16.0f); sf(CAR(-1)+0x5000+68,ty/16.0f); sf(CAR(-1)+0x5000+72,tz/16.0f); sf(CAR(-1)+0x5000+76,0.0f);
   uint32_t CH=TGT+0x2000; sf(CH+224,spd/16.0f); sf(CH+580,burn==0?0.0f:burn==1?0.5f:-0.25f); store(CH+592,(bike&&bd)?0x10u:0u);
   sf(TGT+0x3000+36,wreck==0?10.0f:wreck==1?100.0f:150.0f);
   uint32_t m1=0,mb=0;
   for(int i=0;i<nc;i++){ int ox,oy,oz,ug,wr,br,s1,sb2; in>>ox>>oy>>oz>>ug>>wr>>br>>s1>>sb2; uint32_t cc=CAR(i);
    sf(cc+0x5000+64,(tx+ox)/16.0f); sf(cc+0x5000+68,(ty+oy)/16.0f); sf(cc+0x5000+72,(tz+oz)/16.0f); sf(cc+0x5000+76,0.0f);
    sb(cc+880,ug); sf(cc+0x3000+36,wr==0?10.0f:wr==1?100.0f:150.0f); store(cc+876,br?cc+0x8000:0u); m1|=(uint32_t)s1<<i; mb|=(uint32_t)sb2<<i; }
   store(IN+4,m1); store(IN+8,mb);
   sf(st[6]+16,lost?0.37f:0.0f);
   if(ask>=0) cur=ask;
   // pull-over: 0 CatchUp .. 2 (Ticket is 3); arrow trap: 3 Search, else 2
   int fs = cls==0 ? (cur==6?1:0) : (cur==6?3:2);
   store(F+12,(uint32_t)fs);
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)lf2(dtb); SEEN(c,nullptr);
   c=fresh(); c.r3.u32=M; TEST(c,nullptr); int fire=c.r3.u32&1;
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)lf2(dtb); COMMON(c,nullptr);
   printf("%d %d %d %d %08x %08x %08x %08x %08x\n",k,fs,fs,fire,bits(lf(M+108)),bits(lf(M+140)),bits(lf(M+144)),bits(lf(M+124)),bits(lf(M+132)));
  }
  printf("end\n");}
}
'''


def stubs_for(name, a):
    flag = lambda k: '((load(0x40C00000u)>>%d)&1)' % P.FLAGS.index(k)
    f32s = lambda x: '([](uint32_t u){float f;memcpy(&f,&u,4);return f;})(load(%s))' % x
    stubs = {name(f): 'throw std::runtime_error("unexpected call %s");' % f for f in P.NOCOMPILE + a.more}
    for f, k in [('F-96080', 'moving'), ('F-80249', 'same_dir'), ('F-99390', 'ahead'), ('F-30098', 'on_right'), ('F-43223', 'same_facing'),
                 ('F-90575', 'all_block'), ('F-32902', 'closing'), ('F-14848', 'pick')]:
        stubs[name(f)] = 'ctx.r3.u64=%s;' % flag(k)
    stubs[name('F-90304')] = 'ctx.f1.f64=(double)%s;' % f32s('0x40C00004u')
    stubs[name('F-24671')] = 'uint32_t B=load(ctx.r3.u32+876); store(B+568,ctx.r4.u32);'
    for f in ['F-77791', 'F-72181', 'F-59773', 'F-94949']:
        stubs[name(f)] = ''
    stubs[name('F-29545')] = 'ctx.r3.u64=0;'
    stubs[name('F-14415')] = 'ctx.r3.u64=1;'
    stubs[name('F-94676')] = ('uint32_t i=(ctx.r3.u32-0x40B00000u)>>16; float r=(float)ctx.f1.f64; float bk=' + f32s('0x40D00010u') +
                              '; uint32_t m=load(r==bk?0x40D00008u:0x40D00004u); ctx.r3.u64=(m>>i)&1u;')
    stubs[name('F-16046')] = 'ctx.r3.u64=(load(0x40D00000u)>>24)&1u;'
    stubs[name('F-17373')] = 'ctx.r3.u64=load(0x40D0000Cu)&1u;'
    stubs[name('F-78189')] = ''
    stubs[name('F-33706')] = 'ctx.r3.u64=0;'
    stubs[name('F-12823')] = ''
    stubs[name('F-39048')] = ''
    return stubs


def run_exe(exe, a, cf, ai, pl):
    r = subprocess.run([str(exe), str(a.image), str(cf), '%08X' % ai, '%08X' % pl], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-1500:] + r.stderr[-1500:])
    return r.stdout.split('end\n')[:-1]


def record(seeds, blocks, heads, ref, end_on_fire):
    """the reference lines: a case's rows where something changed, every 20th, the last and the firing one"""
    ticks_n, fires = 0, 0
    for seed, blk, head in zip(seeds, blocks, heads):
        rows = blk.split('\n')
        rows = [x for x in rows[1:] if x]
        ref.append(f'CASE {seed}{head}')
        prev = None
        for k, row in enumerate(rows):
            v = row.split()
            key = v[1:4] + [int(x, 16) == 0 for x in v[4:]]
            if key != prev or k % 20 == 0 or k == len(rows) - 1 or (end_on_fire and v[3] == '1'):
                ref.append(row)
            prev = key
        ticks_n += len(rows)
        fires += sum(r.split()[3] == '1' for r in rows)
    return ticks_n, fires


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True, help='the Pursuit run (arrest_ref.txt)')
    ap.add_argument('--ref-vt', type=Path, required=True, help='the pull-over / arrow trap classes (arrest_vt_ref.txt)')
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--more', nargs='*', default=[], help='more F-IDs to stub with a throw')
    a = ap.parse_args()
    world = [f for f in P.WORLD if f not in ('F-36806', 'F-63286', 'F-41260', 'F-14415', 'F-29545')]
    ids = (MAIN_IDS + ['F-28114', 'F-36806', 'F-63286', 'F-74492', 'F-66453', 'F-41284'] + P.NOCOMPILE + world + P.TARGETS + TARGETS
           + OWN_STUBS + a.more)
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    ai, pl = rdisp.vtables_of(img, 'mcAiRacer')[0][0], rdisp.vtables_of(img, 'mcPlayer')[0][0]
    stubs = stubs_for(name, a)
    fakes = {0x7F000010: 'ctx.r3.u64=0;', 0x7F000011: ''}
    vtabs = [(IMG + 0x0C8764, 16)] + [(v, [i for i in range(21) if i != 15]) for v, _ in P.VTABLES[1:]]
    tgts = [addr[f] for f in P.TARGETS + TARGETS + ['F-66453', 'F-41284']] + RAW_TARGETS
    idx = rdisp.index(a.recomp)
    sub = (lambda s: s.replace('CTOR', name('F-55508')).replace('RESET', name('F-20606')).replace('BEGIN', name('F-89627'))
           .replace('UPDATE', name('F-68032')).replace('ASK(', name('F-28114') + '(').replace('TUNE', '0x%08Xu' % P.TUNE)
           .replace('TACTIC_VT', '0x%08Xu' % TACTIC_VT).replace('MGRG', '0x%08Xu' % (IMG + 0x86D8E0)).replace('CVT0', '0x%08Xu' % (IMG + 0x0C8B54)).replace('CVT1', '0x%08Xu' % (IMG + 0x0C81BC)).replace('SEEN(', name('F-74492') + '(')
           .replace('TEST(', name('F-36806') + '(').replace('COMMON(', name('F-63286') + '(')
           .replace('lf2(dtb)', '([](uint32_t u){float f;memcpy(&f,&u,4);return f;})(dtb)'))
    seeds = [0x9E3779B9 * (i + 11) & 0xFFFFFFFF for i in range(CASES)]
    # --- the Pursuit, through its update
    exe = rdisp.build(idx, [name(f) for f in MAIN_IDS] + [name('F-28114')], sub(MAIN), a.out, 'arrest', image=img, vtables=vtabs,
                      targets=tgts, limit=900, stubs=stubs, fakes=fakes)
    lines = []
    for seed in seeds:
        (dtb, safe, at, thr, biker, n, bike), ticks = gen_case(seed)
        lines.append(f'case {N} {dtb:x} {safe} {at} {thr} {biker} {n} {bike}')
        lines += [' '.join(map(str, t)) for t in ticks]
    cf = a.out / 'arrest_cases.txt'
    cf.write_text('\n'.join(lines) + '\nstop\n')
    blocks = run_exe(exe, a, cf, ai, pl)
    ref = []
    ticks_n, fires = record(seeds, blocks, [''] * len(seeds), ref, True)
    a.ref.write_text('\n'.join(ref) + '\n')
    print('pursuit: cases', len(blocks), 'updates', ticks_n, 'fires', fires, '->', a.ref)
    # --- the pull-over's and the arrow trap's classes: seen update, test and common update called in order
    exe = rdisp.build(idx, [name(f) for f in MAIN_IDS] + [name('F-36806'), name('F-63286')], sub(MAIN_VT), a.out, 'arrest_vt', image=img,
                      vtables=vtabs, targets=tgts, limit=900, stubs=stubs, fakes=fakes)
    lines = []
    for i, seed in enumerate(seeds):
        (dtb, safe, at, thr, biker, n, bike), ticks = gen_case(seed)
        lines.append(f'case {N} {dtb:x} {safe} {at} {thr} {biker} {n} {bike} {i % 2}')
        lines += [' '.join(map(str, t)) for t in ticks]
    cf = a.out / 'arrest_vt_cases.txt'
    cf.write_text('\n'.join(lines) + '\nstop\n')
    blocks = run_exe(exe, a, cf, ai, pl)
    ref = []
    ticks_n, fires = record(seeds, blocks, [f' {i % 2}' for i in range(len(seeds))], ref, False)
    a.ref_vt.write_text('\n'.join(ref) + '\n')
    print('classes: cases', len(blocks), 'updates', ticks_n, 'tests true', fires, '->', a.ref_vt)


if __name__ == '__main__':
    main()
