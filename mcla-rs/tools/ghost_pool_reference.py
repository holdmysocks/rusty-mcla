"""The cruising ghost's ambient request pool run offline: the generated code whole and unchanged, in vector mode, on flat
guest memory. Run whole: the pool step F-52865 with the record step F-29339 and every stage (F-18708 the pose from the
route, F-12577 the car, F-19334 the behaviour, F-42724 the spawn with the shared MWC draw and F-46111, the removal
stages F-10793 / F-73806, the end steps F-31134 / F-92217 / F-59370, the notify F-90613), the queue F-59429 -> F-13129
with its record init F-75521 and dedupe F-83456, the cancel F-11284, the removal request F-51512 -> F-76814, the request
ctor F-86587 and fill F-45833 (with the brain's SetSpeed / SetDestSpeed edits of F-25539), the type budget F-49084 /
F-39893; and alone the integrator F-58614 and the MWC seed F-37317.

Stubbed and logged (the world's side): the car pool F-21879 (get) / F-73520 (give back), the behaviour pool F-60281 /
F-24235, the type's slot 14 probe (F-15062: answered from the case), the car init (car vfunc 5, F-77422: only its stores
the pool reads later, car +452 = the behaviour and car +52 = the owner), the car release (car vfunc 11), the behaviour's
vfunc 0, the owner's vfuncs 35 / 36 (the attach and the removal notify), the lane's slot 8 (the route lane's point and
tangent, from the case); the network send F-35362 and tracker F-26952 empty, the network test F-24338 false. The start-up constant at 0x828609E0 (zero in the image) is
set to +inf as the running game has it.

Writes <out-dir>/ghost_pool_ref.txt, one event a line. Scenarios: "Z <budget> <shared lo> <shared carry>", then ops
"q <owner> <kind> <lane> <t> <set> <dest> <v> = <id>" (kind 1 a route request, 0 the idx -1 request; set / dest the
SetSpeed / SetDestSpeed words or ffffffff for none), "x <id>" (cancel), "r <car> <reason>" (F-51512), "s <dt>" (a pool
step), each followed by what the code did: "C <car>" car pool (0 none), "c <car>" car back, "B <beh>" behaviour pool,
"b <beh>" behaviour back, "D <beh>" behaviour vfunc 0, "L <lane> <t> <p xyz> <tangent xyz>" lane slot 8, "P <car> <seed>
<rows a b c place xyz> <answer>" probe, "I <car> <beh> <seed>" car init, "K <car> <reason>" car release, "N <owner> <vf>
<status> <car>" notify; after a step "S <count> <max> <type +4> <type +8> <shared lo> <shared carry>". Then
"G <inputs> = <outputs>" integrator cases and "M <x> = <lo> <carry>" seed cases. Floats as bit words; a word that would
read as a code address is split into 16-bit halves (xxxx_xxxx)."""
import argparse
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

SCRATCH = 0x400000
# (extsb / extsh read a register's low byte / half signed)
recomp_closure.shim = lambda: recomp_leaf.shim(SCRATCH).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;', 1).replace(
    'throw std::runtime_error("unexpected guest access");', '{char t[48];snprintf(t,48,"unexpected guest access %08x",a);throw std::runtime_error(t);}', 1)
recomp_closure.EXTRA += r'''
#define ctr r2
static void dispatch(Context&,const uint8_t*,uint32_t);
#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)
static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}
namespace rex{namespace ppc{static inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i v){alignas(16) uint32_t u[4];
simde_mm_store_si128((simde__m128i*)u,v);return simde_mm_set_ps(float(u[3]),float(u[2]),float(u[1]),float(u[0]));}}}
static uint32_t RS=1;static uint32_t rnd(){RS^=RS<<13;RS^=RS>>17;RS^=RS<<5;return RS;}
static float fr(){return (rnd()>>8)*(1.0f/16777216.0f);}
static uint32_t fbits(float f){uint32_t u;memcpy(&u,&f,4);return u;}
static int PCAR=85,PBEH=90,PPROBE=90;static uint32_t NEXTCAR,NEXTBEH;static float LANEP[8][6];
static std::vector<uint32_t> LIVE;
#include <cstdlib>
#include <atomic>
#define REX_ENTER_GLOBAL_LOCK()
#define REX_LEAVE_GLOBAL_LOCK()
#define REX_CHECK_GLOBAL_LOCK() 0
#define REX_QUERY_TIMEBASE() 1
static const uint32_t M=0x100000,POOL=M+8800,T=0x168000,VT_T=0x168800,OWN=0x170000,VT_O=0x178000,CAR0=0x180000,VT_C=0x1C8000,
 BEH0=0x1D0000,VT_B=0x1E8000,LANE0=0x1F0000,VT_L=0x1F8000,REQ=0x200000,DESC=0x201000,ROUTE=0x201400,GCAR=0x210000,GT=0x212000,
 GB=0x213000,SEEDO=0x214000;
static const uint32_t F_TV6=0x7F000000,F_TV7=0x7F000004,F_TV14=0x7F000008,F_CV5=0x7F00000C,F_CV11=0x7F000010,F_BV0=0x7F000014,
 F_OV35=0x7F000018,F_OV36=0x7F00001C,F_LS8=0x7F000020;
static void wipe(uint32_t a,uint32_t n){memset(pointer(a),0,n);}
static void imgput(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static uint32_t imgget(uint32_t a){const uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|p[1]<<16|p[2]<<8|p[3];}
static void W3(uint32_t a){printf(" %08x %08x %08x",load(a),load(a+4),load(a+8));}
'''
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/avx2.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
_closure = recomp_closure.closure
recomp_closure.closure = lambda idx, names, stubs=(), limit=400: _closure(idx, names, stubs, limit)
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3', '-mavx2'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def word(t):
    if len(t) == 8 and all(c in '0123456789abcdef' for c in t) and t.startswith('82'):
        return t[:4] + '_' + t[4:]
    return t


# guest memory (scratch 0x1000 .. SCRATCH): the manager, its pool at +8800 and its clock at +0x64238; the type; the owners
# (racers); the cars; the behaviours; the route lanes; the request, descriptor and route record; the integrator's car
MAIN = r'''
static void call_table(Context&ctx,const uint8_t*base,uint32_t a);
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==F_TV6){ctx.r3.u32=0;return;}
 if(a==F_TV7){ctx.r3.u32=7;return;}
 if(a==F_TV14){uint32_t d=ctx.r4.u32;printf("P %08x %08x",load(d+244),load(d+288));for(int k=0;k<4;k++)W3(d+16*k);
  uint32_t ans=rnd()%100<(uint32_t)PPROBE;printf(" %u\n",ans);ctx.r3.u32=ans;return;}
 if(a==F_CV5){uint32_t car=ctx.r3.u32,d=ctx.r4.u32;store(car+452,load(d+248));store(car+52,load(d+260));
  printf("I %08x %08x %08x\n",car,load(d+248),load(d+288));return;}
 if(a==F_CV11){printf("K %08x %08x\n",ctx.r3.u32,ctx.r4.u32);for(size_t k=0;k<LIVE.size();k++)if(LIVE[k]==ctx.r3.u32){LIVE.erase(LIVE.begin()+k);break;}return;}
 if(a==F_BV0){printf("D %08x\n",ctx.r3.u32);ctx.r3.u32=0;return;}
 if(a==F_OV35||a==F_OV36){printf("N %08x %u %08x %08x\n",ctx.r3.u32,a==F_OV35?35:36,ctx.r4.u32,ctx.r5.u32);
  if(a==F_OV35&&ctx.r4.u32==0&&ctx.r5.u32)LIVE.push_back(ctx.r5.u32);return;}
 if(a==F_LS8){uint32_t i=(ctx.r4.u32-LANE0)/0x80;float t=float(ctx.f1.f64);printf("L %u %08x",i,fbits(t));
  for(int k=0;k<6;k++)printf(" %08x",fbits(LANEP[i][k]));printf("\n");
  for(int k=0;k<3;k++){putf(ctx.r3.u32+4*k,LANEP[i][k]);putf(ctx.r6.u32+4*k,LANEP[i][3+k]);}putf(ctx.r3.u32+12,0);putf(ctx.r6.u32+12,0);return;}
 call_table(ctx,base,a);}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--scenarios', type=int, default=300)
    ap.add_argument('--integ', type=int, default=3000)
    a = ap.parse_args()
    ids = ('F-52865', 'F-59429', 'F-11284', 'F-51512', 'F-86587', 'F-45833', 'F-58614', 'F-37317', 'F-18347', 'F-64433',
           'F-11513', 'F-33375', 'F-57400', 'F-21879', 'F-73520', 'F-60281', 'F-24235', 'F-35362', 'F-26952', 'F-24338', 'F-47121', 'F-62311')
    A = {f: addr(a.tool, f) for f in ids}
    S = lambda f: 'sub_' + A[f]
    stubs = {
        S('F-21879'): ('uint32_t ans=rnd()%100<(uint32_t)PCAR?NEXTCAR:0;if(ans){NEXTCAR+=0x1000;wipe(ans,0x1000);store(ans,VT_C);'
                       '*pointer(ans+500)=0x20;store(ans+476,(ans-CAR0)/0x1000+100);store(ans+400,(ans-CAR0)/0x1000+7);}'
                       'printf("C %08x\\n",ans);ctx.r3.u32=ans;'),
        S('F-73520'): 'printf("c %08x\\n",ctx.r4.u32);',
        S('F-60281'): ('uint32_t ans=rnd()%100<(uint32_t)PBEH?NEXTBEH:0;if(ans){NEXTBEH+=0x400;wipe(ans,0x400);store(ans,VT_B);}'
                       'printf("B %08x\\n",ans);ctx.r3.u32=ans;'),
        S('F-24235'): 'printf("b %08x\\n",ctx.r4.u32);',
        S('F-35362'): '', S('F-26952'): '', S('F-47121'): '',
        S('F-24338'): 'ctx.r3.u32=0;',
        # (the lock word's compare-and-swap, lwarx / stwcx.: one thread)
        S('F-62311'): 'uint32_t o=load(ctx.r3.u32);if(o==ctx.r5.u32)store(ctx.r3.u32,ctx.r4.u32);ctx.r3.u64=o;',
    }
    roots = [S(f) for f in ('F-52865', 'F-59429', 'F-11284', 'F-51512', 'F-86587', 'F-45833', 'F-58614', 'F-37317', 'F-18347',
                            'F-64433', 'F-57400')]
    idx = index(a.recomp)
    # the indirect targets the code reaches through real vtables (the params object's slots 0 and 1)
    names = roots + [S('F-11513'), S('F-33375')]
    call = lambda f, setup, after='': ('{Context ctx{};ctx.r1.u32=0x3FFF00;ctx.r13.u32=0x3F0000;' + setup + 'try{%s(ctx,nullptr);}catch(std::exception&e)'
                                       '{printf("EXC %s %%s\\n",e.what());fflush(stdout);exit(1);}' % (S(f), f) + after + '}')
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 int NSC=atoi(argv[2]),NG=atoi(argv[3]);
 imgput(0x8286D8E4,M);
 // (a start-up constant the image holds as zero, as the running game has it: +inf, the normalisation's select)
 for(int k=0;k<4;k++)imgput(0x828609E0+4*k,0x7F800000);
 for(int sc=0;sc<NSC;sc++){RS=0x9E3779B9u*(sc+1)|1;for(int k=0;k<5;k++)rnd();
  wipe(0x100000,0x120000);LIVE.clear();NEXTCAR=CAR0;NEXTBEH=BEH0;
  PCAR=50+rnd()%51;PBEH=70+rnd()%31;PPROBE=60+rnd()%41;
  // the pool constructed by its own ctor (F-57400)
  {CALLPOOL}
  store(T,VT_T);store(VT_T+24,F_TV6);store(VT_T+28,F_TV7);store(VT_T+56,F_TV14);uint32_t budget=1+rnd()%6;store(T+4,budget);
  store(T+8,0);*pointer(T+108)=1;*pointer(T+111)=0;*pointer(T+112)=0;putf(T+124,40.0f);putf(T+136,5.0f);
  store(M+8492,0x150000);
  store(VT_O+140,F_OV35);store(VT_O+144,F_OV36);for(int k=0;k<4;k++)store(OWN+0x100*k,VT_O);
  store(VT_C+20,F_CV5);store(VT_C+44,F_CV11);store(VT_B,F_BV0);store(VT_L+32,F_LS8);
  for(int k=0;k<8;k++){store(LANE0+0x80*k,VT_L);float h=fr()*6.2831853f;float p[6]={(fr()-0.5f)*4000,fr()*40-5,(fr()-0.5f)*4000,
   std::cos(h)*(1+fr()*30),(fr()-0.5f)*(k&1?2.0f:0.0f),std::sin(h)*(1+fr()*30)};for(int j=0;j<6;j++)LANEP[k][j]=p[j];}
  uint32_t slo=rnd(),scy=rnd()&0x7FFF;imgput(0x82860890,slo);imgput(0x82860894,scy);
  printf("Z %08x %08x %08x\n",budget,slo,scy);
  int nops=20+rnd()%60;
  for(int op=0;op<nops;op++){uint32_t p=rnd()%100;
   if(p<40){uint32_t kind=rnd()%10?1:0,owner=OWN+0x100*(rnd()%4),lane=rnd()%8;float t=fr();
    float set=rnd()%2?1.0f+fr()*30:0.0f,dest=rnd()%2?fr()*20:-1.0f,v=(fr()-0.2f)*20;if(rnd()%8==0)dest=0.0f;
    {CALLCTOR}
    if(kind){store(DESC,T);store16(DESC+4,0);store(DESC+8,owner);putf(DESC+12,3.0f);putf(DESC+16,v>0?v:0.0f);store(DESC+20,0xFFFFFFFF);
     store(DESC+24,0xFFFFFFFF);*pointer(DESC+28)=0;{CALLROUTE}store(ROUTE,LANE0+0x80*lane);putf(ROUTE+8,t);store(ROUTE+128,0);{CALLFILL}}
    if(set>0){putf(REQ+68,set);*pointer(REQ+80)|=0x40;}if(dest>-1.0f){putf(REQ+76,dest);*pointer(REQ+80)|=0x20;}
    printf("q %08x %u %u %08x %08x %08x %08x",owner,kind,lane,fbits(t),set>0?fbits(set):0xFFFFFFFFu,dest>-1.0f?fbits(dest):0xFFFFFFFFu,fbits(v>0?v:0.0f));
    uint32_t id=0;{CALLQ}printf(" = %08x\n",id);
   }else if(p<50){std::vector<int> pend;for(int i=0;i<256;i++){uint32_t r=POOL+16+848*i;if(load(r+4)==2&&(*pointer(r+300)&0x80))pend.push_back(i);}
    if(pend.empty())continue;int id=pend[rnd()%pend.size()];printf("x %08x\n",id);{CALLX}
   }else if(p<62){if(LIVE.empty())continue;uint32_t car=LIVE[rnd()%LIVE.size()];uint32_t reason=rnd()%3==0?5:1+rnd()%2;
    printf("r %08x %08x\n",car,reason);{CALLR}
   }else{static const float DT[5]={1.0f/60,1.0f/30,0.25f,0.7f,0.0f};float dt=DT[rnd()%5];printf("s %08x\n",fbits(dt));
    {CALLS}
    printf("S %08x %08x %08x %08x %08x %08x\n",load(POOL),load(POOL+4),load(T+4),load(T+8),imgget(0x82860890),imgget(0x82860894));}
  }
 }
 // the integrator alone
 RS=0x51ED270Bu;
 for(int g=0;g<NG;g++){wipe(GCAR,0x1000);wipe(GT,0x200);wipe(GB,0x400);
  float ms=10+fr()*40,ma=1+fr()*10,v=fr()<0.1f?0.0f:fr()*ms*1.3f,a=(fr()-0.5f)*30,dt=rnd()%4?1.0f/60:fr()*0.1f;if(rnd()%10==0)v=0.1f+fr()*0.2f;
  float set=fr()*45,clock=fr()*500,born=clock-fr()*20,timer=fr()*10;uint32_t hasb=rnd()%10!=0,fl=rnd()%2?0x40:0,pb=rnd()%2?0x20:0;
  uint32_t lo=rnd(),cy=rnd()&0xFFFF;float off=(rnd()%3)*6.0f-6.0f;
  if(rnd()%6==0)a=ma+fr()*3;if(rnd()%8==0)v=ms+(fr()-0.5f);
  store(GCAR+448,GT);store(GCAR+452,hasb?GB:0);putf(GT+124,ms);putf(GT+136,ma);*pointer(GB+256)=fl|(rnd()%2?0x20:0);putf(GB+244,set);
  putf(GCAR+412,v);putf(GCAR+464,born);putf(GCAR+432,timer);store(GCAR+384,lo);store(GCAR+388,cy);*pointer(GCAR+500)=pb;putf(GCAR+424,off);
  putf(M+0x64238,clock);
  printf("G %08x %08x %08x %08x %08x %u %02x %08x %08x %08x %08x %08x %08x %02x %08x",fbits(v),fbits(a),fbits(dt),fbits(ms),fbits(ma),
   hasb,*pointer(GB+256),fbits(set),fbits(clock),fbits(born),fbits(timer),lo,cy,pb,fbits(off));
  {CALLG}
  printf(" = %08x %08x %08x %08x %08x\n",load(GCAR+412),load(GCAR+424),load(GCAR+432),load(GCAR+384),load(GCAR+388));}
 for(int g=0;g<500;g++){uint32_t x=g<3?(uint32_t)g:rnd();if(g%7==0)x&=0xFFFF;wipe(SEEDO,32);{CALLM}
  printf("M %08x = %08x %08x %02x %08x\n",x,load(SEEDO),load(SEEDO+4),*pointer(SEEDO+8),load(SEEDO+12));}
 return 0;}
'''
    rep = {
        '{CALLPOOL}': call('F-57400', 'ctx.r3.u32=POOL;'),
        '{CALLCTOR}': call('F-86587', 'ctx.r3.u32=REQ;'),
        '{CALLROUTE}': call('F-18347', 'ctx.r3.u32=ROUTE;'),
        '{CALLFILL}': call('F-45833', 'ctx.r3.u32=REQ;ctx.r4.u32=DESC;ctx.r5.u32=ROUTE;ctx.r6.u32=0;'),
        '{CALLQ}': call('F-59429', 'ctx.r3.u32=M;ctx.r4.u32=REQ;', 'id=ctx.r3.u32;'),
        '{CALLX}': call('F-11284', 'ctx.r3.u32=POOL;ctx.r4.u32=id;'),
        '{CALLR}': call('F-51512', 'ctx.r3.u32=M;ctx.r4.u32=car;ctx.r5.u32=reason;'),
        '{CALLS}': call('F-52865', 'ctx.r3.u32=POOL;ctx.f1.f64=dt;ctx.r5.u32=M;'),
        '{CALLG}': call('F-58614', 'ctx.r3.u32=GCAR;ctx.f1.f64=a;ctx.f2.f64=dt;'),
        '{CALLM}': call('F-37317', 'ctx.r3.u32=SEEDO;ctx.r4.u32=x;'),
    }
    for k, v in rep.items():
        assert body.count(k) == 1, k
        body = body.replace(k, v)
    # (the kernel imports the allocator path names: empty, one thread; the indirect targets reached through the game's own
    # vtables, added as a run meets them: their addresses are read off the run, not written here)
    while True:
        while True:
            try:
                fs = recomp_closure.closure(idx, names, stubs, limit=400)
                break
            except KeyError as e:
                assert e.args[0].startswith('__imp__'), e
                stubs[e.args[0]] = ''
        table = ('static void call_table(Context&ctx,const uint8_t*base,uint32_t a){switch(a){' +
                 ''.join('case 0x%s:%s(ctx,base);return;' % (n[4:], n) for n in fs if n.startswith('sub_')) +
                 '}char b[64];snprintf(b,64,"unexpected indirect call %08x",a);throw std::runtime_error(b);}')
        main_src = MAIN + table + nl + body
        exe = build(idx, list(fs), main_src, a.out_dir, 'ghost_pool', stubs=stubs, vector=True)
        r = subprocess.run([str(exe), str(a.image), str(a.scenarios), str(a.integ)], capture_output=True, text=True)
        miss = [t.split()[-1] for t in r.stdout.split(nl) if 'unexpected indirect call' in t]
        if r.returncode and miss:
            names.append('sub_' + miss[0].upper())
            continue
        if r.returncode:
            raise RuntimeError(r.stderr[-2000:] + r.stdout[-2000:])
        break
    lines = [' '.join(word(t) for t in x.split()) for x in r.stdout.split(nl) if x]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'ghost_pool_ref.txt').write_text(nl.join(lines) + nl)
    cnt = lambda c: sum(1 for x in lines if x.startswith(c + ' '))
    att = sum(1 for x in lines if x.startswith('N ') and x.split()[2] == '35')
    st = {}
    for x in lines:
        if x.startswith('N '):
            st[x.split()[3]] = st.get(x.split()[3], 0) + 1
    print(len(lines), 'lines:', cnt('Z'), 'scenarios,', cnt('q'), 'requests,', cnt('x'), 'cancels,', cnt('r'), 'removal requests,',
          cnt('s'), 'steps,', att, 'attach notifies (by status word', st, '),', cnt('K'), 'releases,', cnt('P'), 'probes,',
          cnt('G'), 'integrator cases,', cnt('M'), 'seeds')


if __name__ == '__main__':
    main()
