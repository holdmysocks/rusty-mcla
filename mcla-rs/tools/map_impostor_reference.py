"""The plants' impostor rules run offline as generated, written to crates/game/src/testdata/map_impostor_ref.txt for the port's tests
(cameras2-map3). recomp_closure imports tools/memcap.py, so every process of this script (g++, the harnesses) runs in a 4 GB job;
every input file is built here from a bounded list (case counts and type counts are capped below) and the harness refuses a larger one.
One line per case, "<TAG> <inputs> -> <outputs>"; floats are f32 bit patterns in hex.

  TUN  the tuning lookup F-29337 over a table of items (name, MaxRots, ScheduledFrame; item +4 MaxRots, +8 ScheduledFrame) and a name:
       TUN <items: n, then name maxrots sched ...> <name> -> <MaxRots> <ScheduledFrame>
  SET  a type array as the manager holds it (224-byte records), the per-type setup in the loader's order: the low-LOD mark (name holds
       the suffix), the drawable (radius), F-29337 into +144 / +152, then F-64555 (the pictures F-28056 per type with a drawable, then the
       twins linked to their base). The device's target creation is recorded:
       SET <tune: n, name maxrots sched ...> <types: n, name hasdrawable radiusbits ...> -> <creations> | <per type: MaxRots Sched low pic196 pic188 pic192 link>
         creation = name:<r5>:<w>:<h>:<r8>:<12 descriptor words>; pic = 0 or the 1-based number of the creation that made it;
         link = -1 or the index of the base type (+212)
  DUE  F-65228 (the per-frame update) with F-27437 and F-44853 run as generated, frames in a row; the cards' stamps are set between
       frames by the case (types seen in a frame get the frame counter):
       DUE <n types: has168 has196 low maxrots sched b217 twin(-1 or index) ...> <f435> <night per frame...> -> per frame: <events> / <b217 per type>
  RRB  F-80686 (shadow impostors) with F-61167 run as generated, hour after hour:
       RRB <n types: has168 has196 low ...> <index0> <last hour> <hours...> -> per call: <events> idx=<index> last=<hour bits>
  BND  F-48104: which pictures a card binds: BND <shadow> <effect begins> <colour> <normal> <shadow picture> -> <events>
  WND  F-65228's wind vector: WND <clock bits> -> <4 words>
Usage: map_impostor_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE --types FILE [--tool mcla-tool]
  --types: the shipped type list, one line per record of the foliage resource in order: "<name> <1 if the name cut at the suffix is a prop
  with a drawable> <that drawable's bounding-sphere radius, f32 bits in hex>" (written by the port's parser: the ignored test
  `props::impostor::data_tests::dump_type_list`, MCLA_DUMP_TYPES=<file>); --tune: tune/MultiPropTuning.xml."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

MAX_TYPES = 512

# the SDK's intrinsics the vector code of F-65228 needs (vctsxs, vrlimi128's permute), as the other reference scripts carry them
VEC_EXTRA = (
    '#define simde_mm_permute_ps(a,imm) simde_mm_shuffle_ps(a,a,imm)\n'
    'namespace rex { namespace ppc {\n'
    'inline simde__m128i simde_mm_vctsxs(simde__m128 src1) {\n'
    '  simde__m128 xmm2 = simde_mm_cmpunord_ps(src1, src1);\n'
    '  simde__m128i xmm0 = simde_mm_cvttps_epi32(src1);\n'
    '  simde__m128i xmm1 = simde_mm_cmpeq_epi32(xmm0, simde_mm_set1_epi32(INT32_MIN));\n'
    '  xmm1 = simde_mm_andnot_si128(simde_mm_castps_si128(src1), xmm1);\n'
    '  simde__m128 dest = simde_mm_blendv_ps(simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(simde_mm_set1_epi32(INT32_MAX)), simde_mm_castsi128_ps(xmm1));\n'
    '  return simde_mm_andnot_si128(simde_mm_castps_si128(xmm2), simde_mm_castps_si128(dest));\n'
    '} } }\n')
MAX_TUNE = 256


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def hi(x):
    return (x << 16) & 0xFFFFFFFF


G_TUN = hi(-32119) - 12124
G_MGR = hi(-32119) - 12128
G_DEPTH = hi(-32119) - 12120
G_SUB = hi(-32119) - 12112
G_OFF = hi(-32119) - 12068
G_DEV = hi(-32124) - 25232
G_WHITE = hi(-32124) - 25360
G_VP = hi(-32124) - 26876
G_FRAME = hi(-32131) + 11724
G_LIM = hi(-32131) + 17320
G_RR = hi(-32119) - 12188
G_HOUR = hi(-32119) - 12168
G_PERIOD = hi(-32119) - 12000
G_AMP = hi(-32119) - 11952
G_HANDLE = hi(-32119) - 12156
G_RN = hi(-32120) - 8092
GDT = hi(-32131) + 29952
G_TECH_DAY = hi(-32130) + 22732
G_TECH_NIGHT = hi(-32130) + 22740
G_TECH_SHADOW = hi(-32130) + 22736

COMMON = r'''
#include <cstdarg>
static std::string LOG;
static void ev(const char*f,...){char b[512];va_list a;va_start(a,f);vsnprintf(b,sizeof b,f,a);va_end(a);LOG+=b;LOG+=' ';}
static std::string gcs(uint32_t a){std::string s;for(;*pointer(a);a++)s+=char(*pointer(a));return s;}
static uint32_t NEXTPIC;
static uint32_t g_night, g_hour_bits;
static const uint32_t TUN=0x40100000u,ARR=0x40100100u,ITEMS=0x40101000u,NAMES=0x40110000u,MGR=0x40120000u,LIST=0x40120100u,
  TYPES=0x40130000u,DRW=0x401B0000u,POOL=0x401C0000u,DEVO=0x401D0000u,DVT=0x401D0100u,VP=0x401E0000u,LIGHT=0x401E1000u,SCR=0x401F0000u;
static const uint32_t FAKE_CREATE=0x7F0010E0u,FAKE_SETRT=0x7F0010F0u,FAKE_CLEAR=0x7F001100u;
static void wipe(){std::memset(pointer(0x40100000u),0,0x100000u);}
static uint32_t put_str(uint32_t&top,const std::string&s){if(top+s.size()+8>0x40120000u)throw std::runtime_error("name pool full");uint32_t a=top;for(char c:s)*pointer(top++)=uint8_t(c);*pointer(top++)=0;top=(top+3)&~3u;return a;}
'''

# the pieces both type-array harnesses share: reading a case's tune table and type list into guest memory
MAIN_COMMON = r'''
// (the build's own option hook in the shadow impostor function: the shipped default, foliage shadows on, takes the original path)
bool mcla_foliage_shadows(PPCRegister&){return false;}
static uint32_t pool_top;
static uint32_t read_tune(std::istream&in){
 uint32_t n;in>>std::dec>>n;if(n>MAX_TUNE_)throw std::runtime_error("tune table too long");
 pool_top=NAMES;
 for(uint32_t i=0;i<n;i++){std::string nm;uint32_t mr,sc;in>>nm>>mr>>sc;uint32_t it=ITEMS+16*i;store(it,put_str(pool_top,nm));store(it+4,mr);store(it+8,sc);store(ARR+4*i,it);}
 store(TUN+4,ARR);store16(TUN+8,n);store16(TUN+10,n);
 return n;}
static void make_device(){store(DEVO,DVT);store(DVT+56,FAKE_CREATE);store(DVT+60,FAKE_SETRT);store(DVT+64,FAKE_CLEAR);store(G_DEV_,DEVO);
 store(G_VP_,VP);store(G_RN_,SCR);store(G_TECH_DAY_,0xAAAA0001u);store(G_TECH_NIGHT_,0xAAAA0002u);store(G_TECH_SHADOW_,0xAAAA0003u);store(G_LIM_,3);
 float per[4]={4.0f,4.0f,5.0f,4.0f},amp[4]={0.075f,0.075f,0.075f,0.4f};
 for(int i=0;i<4;i++){uint32_t b;std::memcpy(&b,&per[i],4);store(G_PERIOD_+4*i,b);std::memcpy(&b,&amp[i],4);store(G_AMP_+4*i,b);}}
'''

MAIN_SET = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 while(in>>k){rt_reset();wipe();LOG.clear();NEXTPIC=0x50000000u;
  make_device();store(G_TUN_,TUN);store(G_SUB_,MGR+0x800);store(G_DEPTH_,0);store(G_OFF_,0);
  read_tune(in);
  uint32_t nt;in>>std::dec>>nt;if(nt>MAX_TYPES_)throw std::runtime_error("type list too long");
  store(MGR+16,LIST);store(LIST+8,TYPES);store16(LIST+12,nt);
  for(uint32_t i=0;i<nt;i++){std::string nm;uint32_t has,rad;in>>nm>>has>>std::hex>>rad>>std::dec;uint32_t t=TYPES+224*i;
   uint32_t np=put_str(pool_top,nm);store(t+128,np);
   if(nm.find("_displayAsLowLOD")!=std::string::npos)*pointer(t+218)=1;
   if(has){uint32_t d=DRW+256*i;store(d+112,rad);store(t+168,d);}
   {Context cx=fresh();cx.r3.u32=TUN;cx.r4.u32=np;cx.r5.u32=t+144;cx.r6.u32=t+152;sub_TUNE(cx,nullptr);}
  }
  {Context cx=fresh();cx.r3.u32=MGR;cx.r4.u32=0;sub_SETUP(cx,nullptr);}
  printf("%s| ",LOG.c_str());
  for(uint32_t i=0;i<nt;i++){uint32_t t=TYPES+224*i;
   auto pic=[&](uint32_t v){return v?(int)((v-0x50000000u)/0x100u+1):0;};
   uint32_t lk=load(t+212);
   printf("%u %u %u %d %d %d %d%s",load(t+144),load(t+152),(unsigned)*pointer(t+218),pic(load(t+196)),pic(load(t+188)),pic(load(t+192)),lk?(int)((lk-TYPES)/224):-1,i+1<nt?" ; ":"");}
  printf("\n");
 }
}
'''

MAIN_TUN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 while(in>>k){rt_reset();wipe();
  read_tune(in);
  std::string q;in>>q;uint32_t np=put_str(pool_top,q);
  store(SCR,0xDEADBEEFu);store(SCR+4,0xDEADBEEFu);
  Context cx=fresh();cx.r3.u32=TUN;cx.r4.u32=np;cx.r5.u32=SCR;cx.r6.u32=SCR+4;sub_TUNE(cx,nullptr);
  printf("%d %u\n",(int)load(SCR),load(SCR+4));
 }
}
'''

MAIN_DUE = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 while(in>>k){rt_reset();wipe();LOG.clear();
  make_device();
  uint32_t nt;in>>std::dec>>nt;if(nt>MAX_TYPES_)throw std::runtime_error("type list too long");
  store(MGR+16,LIST);store(LIST+8,TYPES);store16(LIST+12,nt);
  std::vector<int> twin(nt);
  for(uint32_t i=0;i<nt;i++){uint32_t h168,h196,low,mr,sc,b217;int tw;in>>h168>>h196>>low>>mr>>sc>>b217>>tw;uint32_t t=TYPES+224*i;twin[i]=tw;
   if(h168)store(t+168,DRW+256*i);
   if(h196){store(t+196,0x50000000u+0x100*(3*i+1));store(t+188,0x50000000u+0x100*(3*i+2));store(t+192,0x50000000u+0x100*(3*i+3));}
   *pointer(t+218)=low;store(t+144,mr);store(t+152,sc);*pointer(t+217)=b217;
   if(tw>=0)store(t+212,TYPES+224*tw);}
  uint32_t f435,counter,seen,nf;in>>f435>>counter>>nf;
  store(G_FRAME_,counter);
  for(uint32_t f=0;f<nf;f++){
   uint32_t nightf,visible;in>>nightf>>std::hex>>visible>>std::dec;
   g_night=nightf;*pointer(SCR+435)=f435;store(G_FRAME_,counter+f);
   LOG.clear();
   {Context cx=fresh();cx.r3.u32=MGR;sub_UPDATE(cx,nullptr);}
   printf("%s/",LOG.c_str());
   for(uint32_t i=0;i<nt;i++)printf(" %u:%u",(unsigned)*pointer(TYPES+224*i+217),(unsigned)*pointer(TYPES+224*i+216));
   printf(" |");
   // the scene pass: the types seen in this frame get the counter (a twin's cards stamp its base)
   for(uint32_t i=0;i<nt;i++)if(visible>>i&1){uint32_t t=TYPES+224*i;uint32_t b=load(t+212);store((b?b:t)+156,counter+f);}
   printf(" ;");
  }
  printf("\n");
 }
}
'''

MAIN_RRB = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 while(in>>k){rt_reset();wipe();LOG.clear();
  make_device();
  uint32_t nt;in>>std::dec>>nt;if(nt>MAX_TYPES_)throw std::runtime_error("type list too long");
  store(MGR+16,LIST);store(LIST+8,TYPES);store16(LIST+12,nt);
  for(uint32_t i=0;i<nt;i++){uint32_t h168,h196,low;in>>h168>>h196>>low;uint32_t t=TYPES+224*i;
   if(h168)store(t+168,DRW+256*i);
   if(h196){store(t+196,0x50000000u+0x100*(3*i+1));}
   *pointer(t+218)=low;store(t+144,1);}
  uint32_t idx0,last,nh;in>>std::dec>>idx0>>std::hex>>last>>std::dec>>nh;
  store(G_RR_,idx0);store(G_HOUR_,last);
  store(LIGHT,0x3f000000u);store(LIGHT+4,0xbf800000u);store(LIGHT+8,0x3e800000u);store(LIGHT+12,0);
  for(uint32_t h=0;h<nh;h++){uint32_t hb;in>>std::hex>>hb>>std::dec;g_hour_bits=hb;LOG.clear();
   {Context cx=fresh();cx.r3.u32=MGR;cx.r4.u32=LIGHT;sub_ROUND(cx,nullptr);}
   printf("%s idx=%d last=%08x |",LOG.c_str(),(int)load(G_RR_),load(G_HOUR_));}
  printf("\n");
 }
}
'''

MAIN_BND = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 while(in>>k){rt_reset();wipe();LOG.clear();
  make_device();
  uint32_t sh,ok,c,n,s;in>>std::hex>>sh>>ok>>c>>n>>s>>std::dec;
  store(G_MGR_,MGR);store(MGR+20,MGR+0x100);store(MGR+24,0x11);store(MGR+28,0x22);store(MGR+0x100+20,MGR+0x200);
  store(G_WHITE_,0xFEED0000u);store(G_SUB_,MGR+0x300);
  uint32_t t=TYPES;store(t+196,s);store(t+188,c);store(t+192,n);store(t+200,MGR+0x400);store(MGR+0x400+12,0x77);
  g_night=ok;
  Context cx=fresh();cx.r3.u32=t;cx.r4.u32=sh;cx.f1.f64=0.0;sub_BIND(cx,nullptr);
  printf("%s ret=%u\n",LOG.c_str(),cx.r3.u32&0xFF);
 }
}
'''

MAIN_WND = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 while(in>>k){rt_reset();wipe();LOG.clear();
  make_device();
  store(MGR+16,LIST);store(LIST+8,TYPES);store16(LIST+12,0);
  uint32_t tb;in>>std::hex>>tb>>std::dec;store(GDT_+24,tb);store(G_FRAME_,1);
  {Context cx=fresh();cx.r3.u32=MGR;sub_UPDATE(cx,nullptr);}
  printf("%s\n",LOG.c_str());
 }
}
'''


def fill(main, **kw):
    main = main.replace('RT_THREAD_', '0x%08Xu' % rd.RT_THREAD)
    for k, v in sorted(kw.items(), key=lambda kv: -len(kv[0])):
        main = re.sub(r'\b' + k + r'\b', '0x%08Xu' % v, main)
    return main


def run_exe(exe, image, cases):
    p = subprocess.run([str(exe), str(image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    return p.stdout.splitlines()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--types', type=Path, required=True)
    ap.add_argument('--tune', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--seed', type=int, default=28056)
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    a.work.mkdir(parents=True, exist_ok=True)

    def name(fid):
        return 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    rnd = random.Random(a.seed)
    out = []

    def go(exe, tag, cases, shrink=None):
        assert len(cases) < 100000
        cf = a.work / f'{tag}_cases.txt'
        cf.write_text('\n'.join(cases) + '\n')
        res = run_exe(exe, a.image, cf)
        assert len(res) == len(cases), (tag, len(res), len(cases))
        for c, o in zip(cases, res):
            out.append(f'{c} -> {shrink(o) if shrink else o}')

    def shrink_set(o):      # (a creation: name, r5, width, height, bits per pixel, descriptor word 2 and word 10 (the format code))
        head, rows = o.split('| ')
        cr = []
        for c in head.split():
            p = c.split(':')
            d = p[5].split(',')
            cr.append(':'.join(p[:5] + [d[2], d[10]]))
        return ' '.join(cr) + ' | ' + rows

    def shrink_due(o):      # (the draws only: B<type>:<rotation>; the types' last-night byte after each frame)
        frames = []
        for fr in o.split(' ;'):
            if not fr.strip():
                continue
            ev, st = fr.split('/')
            b = [f"B{m.group(1)}:{m.group(2)}" for m in (re.match(r'B(\d+):(-?\d+):', x) for x in ev.split()) if m]
            frames.append(' '.join(b) + ' /' + ' '.join(s.split(':')[0] for s in st.replace('|', '').split()))
        return ' ; '.join(frames)

    def shrink_rrb(o):      # (the wind zeroed, the calls on the types: B<type>:v for the vectors only, B<type>:d for a picture drawn; the index and last hour)
        calls = []
        for c in o.split('|'):
            if not c.strip():
                continue
            ev = c.split()
            k = []
            for x in ev:
                m = re.match(r'W:\w+:\d+:\d+:(\w+),(\w+),(\w+),(\w+)$', x)
                if m:
                    k.append('W0' if set(m.groups()) == {'00000000'} else 'W?')
                m = re.match(r'B(\d+):-1:\d+:(\d+):', x)
                if m:
                    k.append(f"B{m.group(1)}:{'v' if m.group(2) == '1' else 'd'}")
            k += [x for x in ev if x.startswith('idx=') or x.startswith('last=')]
            calls.append(' '.join(k))
        return ' | '.join(calls)

    consts = dict(G_TUN_=G_TUN, G_MGR_=G_MGR, G_DEPTH_=G_DEPTH, G_SUB_=G_SUB, G_OFF_=G_OFF, G_DEV_=G_DEV, G_WHITE_=G_WHITE, G_VP_=G_VP,
                  G_FRAME_=G_FRAME, G_LIM_=G_LIM, G_RR_=G_RR, G_HOUR_=G_HOUR, G_PERIOD_=G_PERIOD, G_AMP_=G_AMP, G_RN_=G_RN, GDT_=GDT,
                  G_TECH_DAY_=G_TECH_DAY, G_TECH_NIGHT_=G_TECH_NIGHT, G_TECH_SHADOW_=G_TECH_SHADOW,
                  MAX_TYPES_=MAX_TYPES, MAX_TUNE_=MAX_TUNE)
    rc.EXTRA += COMMON.replace('MAX_TYPES_', str(MAX_TYPES))
    rc.EXTRA += VEC_EXTRA + 'union PPCRegister; bool mcla_foliage_shadows(PPCRegister&);' + chr(10)

    def build(tag, main_src, fids, stubs, fakes=None):
        src = fill(MAIN_COMMON + main_src, **consts)
        for k, v in (('sub_TUNE', 'F-29337'), ('sub_SETUP', 'F-64555'), ('sub_UPDATE', 'F-65228'), ('sub_ROUND', 'F-80686'), ('sub_BIND', 'F-48104')):
            src = src.replace(k + '(', name(v) + '(')
        return rd.build(idx, [name(f) for f in fids], src, a.work, tag, image=img, stubs=stubs, fakes=fakes)

    # the device's virtual calls (create target, set target, clear): host bodies at addresses outside the image
    fakes = {
        0x7F0010E0: ('{std::string nm=gcs(ctx.r4.u32);char b[400];snprintf(b,sizeof b,"%s:%u:%u:%u:%u:",nm.c_str(),ctx.r5.u32,ctx.r6.u32,ctx.r7.u32,ctx.r8.u32);'
                     'LOG+=b;for(int i=0;i<12;i++){snprintf(b,sizeof b,"%s%x",i?",":"",load(ctx.r9.u32+4*i));LOG+=b;}LOG+=" ";'
                     'ctx.r3.u32=NEXTPIC;NEXTPIC+=0x100u;}'),
        0x7F0010F0: 'ev("RT:%u:%x:%u",ctx.r4.u32,ctx.r5.u32,ctx.r6.u32);',
        0x7F001100: 'ev("CL:%u:%u:%d",ctx.r4.u32,ctx.r5.u32,(int)ctx.r6.u32);',
    }
    # ---- TUN -------------------------------------------------------------------------------------------------------------------
    tune_items = []
    for m in re.finditer(r'<Name content="ascii">(.*?)</Name>\s*<MaxRots value="(\d+)"/>\s*<ScheduledFrame value="(\d+)"/>', a.tune.read_text()):
        tune_items.append((m.group(1), int(m.group(2)), int(m.group(3))))
    assert len(tune_items) == 29, len(tune_items)
    exe = build('map_imp_tun', MAIN_TUN, ['F-29337'], {})

    def tune_str(items):
        return str(len(items)) + ''.join(f' {n} {m} {s}' for n, m, s in items)
    cases = []
    names = [n for n, _, _ in tune_items]
    qs = list(names) + [n.upper() for n in names] + [n.swapcase() for n in names] + [n + '_displayAsLowLOD' for n in names] + [n[:-1] for n in names] \
        + [n + 'x' for n in names] + ['x' + n for n in names]
    qs += ['la_prop_gfx_hedge_09x', 'LA_PROP_GFX_HEDGE_', 'a_gfx_hedge_', 'gfx_hedge_', 'x_gfx_hedge_y_displayAsLowLOD', 'la_prop_gfx_Hedge_01x', 'la_prop_tree', 'q', 'a', '_gfx_hedge_']
    for q in qs:
        cases.append(f'TUN {tune_str(tune_items)} {q}')
    for _ in range(1500):
        n = rnd.choice([0, 1, 2, 5, 12, 29])
        items = [(rnd.choice(names) if rnd.random() < 0.5 else 'p%d_%s' % (rnd.randrange(50), rnd.choice(['gfx_hedge_x', 'tree', 'Palm', 'BUSH'])),
                  rnd.choice([1, 1, 2, 250, 0, 251, 7]), rnd.choice([0, 1, 2, 4, 8, 3, 15, 255])) for _ in range(n)]
        q = rnd.choice([i[0] for i in items] + ['zz', 'ZZ_gfx_hedge_', 'p3_Palm'] if items else ['zz', 'p3_Palm', 'a_gfx_hedge_b'])
        if rnd.random() < 0.3:
            q = q.upper() if rnd.random() < 0.5 else q.lower()
        cases.append(f'TUN {tune_str(items)} {q}')
    go(exe, 'TUN', cases)

    # ---- SET -------------------------------------------------------------------------------------------------------------------
    stubs = {name(f): '' for f in ('F-20190', 'F-56588', 'F-35685', 'F-15488', 'F-42675')}   # (addref of a shared picture, the one-time manager creation: not reached with the sub-object present)
    exe = build('map_imp_set', MAIN_SET, ['F-64555', 'F-29337'], stubs, fakes)
    real = []
    for line in a.types.read_text().splitlines():
        n, has, rad = line.split()
        real.append((n, int(has), int(rad, 16)))
    assert 0 < len(real) <= MAX_TYPES

    def type_str(types):
        return str(len(types)) + ''.join(f' {n} {h} {r:08x}' for n, h, r in types)
    cases = [f'SET {tune_str(tune_items)} {type_str(real)}']
    pool = ['la_prop_gfx_tree_03x', 'la_prop_gfx_hedge_01x', 'la_prop_gfx_hedge_large_01x', 'la_prop_gfx_bush_01x', 'la_prop_gfx_zz_gfx_hedge_9x',
            'other_gfx_hedge_7', 'plain_tree', 'la_prop_gfx_palm_tree_01x']
    for _ in range(3000):
        nb = rnd.randrange(1, 7)
        tn = [(n, rnd.choice([1, 1, 1, 2, 3, 249, 250, 251, 400, 0]), rnd.choice([0, 1, 2, 4, 8, 3])) for n in rnd.sample(pool, rnd.randrange(0, 5))]
        types = []
        for _ in range(nb):
            b = rnd.choice(pool)
            kind = rnd.random()
            nm = b + '_displayAsLowLOD' if kind < 0.35 else b
            rad = rnd.choice([rnd.uniform(0.2, 1.9), rnd.uniform(1.0, 6.0), 2.0, 2.0000002, 1.9999999, 0.0, 12.5, 1.5, 100.0, -1.0])
            types.append((nm, rnd.choice([1, 1, 1, 0]), fb(rad)))
        cases.append(f'SET {tune_str(tn)} {type_str(types)}')
    go(exe, 'SET', cases, shrink_set)

    # ---- DUE ------------------------------------------------------------------------------------------------------------------
    due_stubs = {
        name('F-47292'): 'ctx.r3.u32=g_night;',
        name('F-32053'): 'ev("W:%x:%u:%u:%08x,%08x,%08x,%08x",ctx.r3.u32,ctx.r5.u32,ctx.r6.u32,load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r4.u32+12));',
        name('F-38438'): 'ctx.r3.u32=0x1000u+ctx.r3.u32;',
        name('F-33110'): '',
        name('F-96199'): '',
        name('F-44853'): 'ev("V%d",(int)((ctx.r3.u32-TYPES)/224));',
        name('F-73098'): 'ev("B%d:%d:%u:%u",(int)((ctx.r3.u32-TYPES)/224),(int)ctx.r5.u32,ctx.r6.u32?1u:0u,ctx.r7.u32&0xFF);',
        name('F-42026'): 'ev("P%d:%u:%u",(int)((ctx.r3.u32-TYPES)/224),ctx.r5.u32&0xFF,ctx.r6.u32&0xFF);',
        name('F-78833'): 'ev("C:%u:%u:%u:%u:%u:%u",ctx.r3.u32,ctx.r4.u32,ctx.r5.u32,ctx.r7.u32,ctx.r8.u32,uint32_t(ctx.f1.f64*1000));',
        name('F-37365'): 'ctx.r3.u32=0x77u;',
        name('F-62018'): 'ev("T:%x",ctx.r3.u32);',
        name('F-86283'): 'ev("U:%u:%u",ctx.r3.u32,ctx.r4.u32);',
        name('F-68992'): '',
    }
    exe = build('map_imp_due', MAIN_DUE, ['F-65228', 'F-27437'], due_stubs, fakes)
    cases = []
    for _ in range(2500):
        nt = rnd.randrange(1, 9)
        types = []
        for i in range(nt):
            tw = -1
            low = rnd.choice([0, 0, 0, 1])
            if low and nt > 1 and rnd.random() < 0.8:
                tw = rnd.choice([j for j in range(nt) if j != i])
            types.append(f"{rnd.choice([1, 1, 1, 0])} {rnd.choice([1, 1, 1, 0])} {low} {rnd.choice([1, 1, 1, 0, 2, 3, 7])} {rnd.choice([0, 1, 2, 4, 8, 3, 5, 15])} {rnd.choice([0, 1, 1, 0, 2, 255])} {tw}")
        nf = rnd.randrange(1, 9)
        counter = rnd.choice([1, 2, 3, 100, 0x7FFFFFFE, 0xFFFFFFFE, 0xFFFFFFFF, 0, rnd.randrange(0, 1 << 32)])
        frames = ' '.join(f'{rnd.choice([0, 0, 1, 1, 2, 255])} {rnd.getrandbits(nt):x}' for _ in range(nf))
        cases.append(f'DUE {nt} ' + ' '.join(types) + f' {rnd.choice([0, 0, 1])} {counter} {nf} {frames}')
    go(exe, 'DUE', cases, shrink_due)

    # ---- RRB ------------------------------------------------------------------------------------------------------------------
    rr_stubs = dict(due_stubs)
    rr_stubs[name('F-22302')] = '{float f;std::memcpy(&f,&g_hour_bits,4);ctx.f1.f64=f;}'
    rr_stubs[name('F-32053')] = 'ev("W:%x:%u:%u:%08x,%08x,%08x,%08x",ctx.r3.u32,ctx.r5.u32,ctx.r6.u32,load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r4.u32+12));'
    rr_stubs[name('F-73098')] = ('ev("B%d:%d:%u:%u:%08x,%08x,%08x,%08x",(int)((ctx.r3.u32-TYPES)/224),(int)ctx.r5.u32,ctx.r6.u32?1u:0u,ctx.r7.u32&0xFF,'
                                 'load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r4.u32+12));')
    rr_stubs[name('F-82266')] = ''
    rr_stubs[name('F-95742')] = ''
    exe = build('map_imp_rrb', MAIN_RRB, ['F-80686', 'F-61167'], rr_stubs, fakes)
    cases = []
    for _ in range(2000):
        nt = rnd.randrange(1, 10)
        tl = [(rnd.choice([1, 1, 1, 0]), rnd.choice([1, 1, 0]), rnd.choice([0, 0, 1])) for _ in range(nt)]
        if not any(x[0] and x[1] for x in tl):     # (the original's search for the next type has no bound: one type must qualify)
            tl[rnd.randrange(nt)] = (1, 1, rnd.choice([0, 1]))
        types = ' '.join(f'{x} {y} {z}' for x, y, z in tl)
        t0 = rnd.choice([-100.0, 0.0, 6.0, 12.5, 23.99, rnd.uniform(0, 24)])
        hrs, h = [], t0
        for _ in range(rnd.randrange(1, 14)):
            step = rnd.choice([0.0, 0.001, 0.008, 1 / 120, 0.0083333, 0.0083334, 0.01, 0.5, -0.004, -0.5, 24.0, -24.0, rnd.uniform(-1, 1)])
            h = h + step
            if rnd.random() < 0.1:
                h = rnd.choice([0.0, 23.999, 24.0, 0.001, 12.0, float('nan')])
            hrs.append(h)
        cases.append(f'RRB {nt} {types} {rnd.choice([0, 0, 1, 2, rnd.randrange(0, 12), 1 << 20, 0xFFFFFFFF])} {fb(t0):08x} {len(hrs)} ' + ' '.join(f'{fb(x):08x}' for x in hrs))
    go(exe, 'RRB', cases, shrink_rrb)

    # ---- BND ------------------------------------------------------------------------------------------------------------------
    bnd_stubs = {
        name('F-56814'): 'ev("S:%x:%x",ctx.r5.u32,ctx.r6.u32);',
        name('F-57125'): 'ev("K");',
        name('F-91379'): 'ctx.r3.u32=g_night;',
        name('F-24298'): 'ev("Q:%u:%x",ctx.r3.u32,ctx.r6.u32);',
        name('F-61918'): 'ev("E");',
    }
    exe = build('map_imp_bnd', MAIN_BND, ['F-48104'], bnd_stubs, fakes)
    cases = [f'BND {sh:x} {ok:x} c1c1c1c1 e2e2e2e2 a3a3a3a3' for sh in (0, 1, 2, 255, 256) for ok in (0, 1)]
    cases += [f'BND {sh:x} {ok:x} 0 {n:x} {s:x}' for sh in (0, 1) for ok in (0, 1) for n in (0, 5) for s in (0, 7)]
    go(exe, 'BND', cases)

    # ---- WND ------------------------------------------------------------------------------------------------------------------
    wnd_stubs = dict(due_stubs)
    wnd_stubs[name('F-73098')] = ''
    exe = build('map_imp_wnd', MAIN_WND, ['F-65228'], wnd_stubs, fakes)
    ts = [0.0, 0.5, 1.0, 2.0, 3.99, 4.0, 5.0, 100.0, 12345.678, 1e6, 65536.5, 16777216.0, -1.0, -4.0, -0.5, 1e-7, 0.001]
    ts += [rnd.uniform(0, 8) for _ in range(500)] + [rnd.uniform(0, 4000) for _ in range(500)] + [rnd.uniform(0, 1e6) for _ in range(300)] + [rnd.uniform(-100, 100) for _ in range(100)]
    go(exe, 'WND', [f'WND {fb(t):08x}' for t in ts])

    keep = [l for l in out if not re.search(r'(?<![0-9a-zA-Z])82[0-9a-f]{6}(?![0-9a-zA-Z])', l)]
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(keep), 'of', len(out), 'cases')


if __name__ == '__main__':
    main()
