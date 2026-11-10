"""The map's view-dependent rules run offline as generated, written to crates/game/src/testdata/map_reach_ref.txt for the
port's tests. recomp_closure imports tools/memcap.py, so every process of this script (g++, the harness) runs in a 4 GB job.
One line per case, "<TAG> <inputs> -> <outputs>"; floats are f32 bit patterns in hex.

  RCH  the plant manager's per-type fade distances F-63362 with its fade bands F-98063, the paused getter F-31789 and the
       original's own tan (all unchanged, as generated). Stubbed (the port has no counterpart, and they do not feed the
       distances): the shader-parameter setter F-32053 (records its calls: slot, count, kind, the four floats), the culling-tree
       walk F-93617, the card pass F-48104 (returns 0: the tail after the bands is skipped), F-84808, F-42781. The type object,
       the drawable (+112 the radius), the view (+696 lens degrees), the plant manager (+32 the night factor) and the renderer
       (+373, +432, +433, the thread block's byte 72) are built by hand.
       RCH <maxrots> <radius> <scale> <lowlod> <fov deg> <night> <f432> <f433> <f373> <tls72> <pass>
           -> <+132> <+136> <+140> <calls> [<kind> <count> <x> <y> <z> <w>]... <current type set> <walks> <card passes>
  FAD  F-98063 alone: FAD <maxrots> <+132> <+136> <scale> <r5> <r6> -> <x> <y> <z> <w> <returned f1>
  PAU  the setter F-65267 (when +432 would be set) and the getter F-31789: PAU <r4> <set> <f373> <tls72> -> <flag>
  OCC  the wrappers F-71608 and F-87100 over a stubbed F-38575 that answers the case's word (the callee is verified apart):
       OCCA <word> <x y z w> -> <visible> <r6 passed> <sphere passed (4 words)> <sphere pointer ok>
       OCCB <word> <flags> <x y z> <radius> -> <word> <r6> <sphere passed (4 words)> <sphere pointer ok>
  LNS  F-50083 with its callees stubbed (the sector directory work, the streaming, the time base): the lens scale and the
       detail distance: LNS <fov deg> <scale> <narrow end> <rate> <dt> <base> <f432> <f433> <f373> <tls72> -> <scale'> <distance>
       (mcla_city_lod, the build's own option hook, is the identity here: the shipped default 1.0)
  FDE  F-89177: FDE <state> <dist> <thr> <t> <flags> <fade state> <choice> <dt> <cross> <detail on> <snap> <far present>
           -> <fade state> <t> <choice>
Usage: map_reach_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def hi(x):
    return (x << 16) & 0xFFFFFFFF


# globals the generated functions read (high half << 16 plus the displacement of the instruction pair)
G_MGR = hi(-32119) - 12128        # the plant manager pointer (night factor at +32)
G_CUR = hi(-32119) - 12100        # the current type, stored by F-63362
G_VP = hi(-32124) - 26876         # a viewport pointer (+696 lens degrees)
G_RN = hi(-32120) - 8092          # the renderer pointer
G_OCC = hi(-32119) - 17628        # the occluder list pointer F-71608 passes
G_VIEW = hi(-32130) + 22080       # the view index word
D = hi(-32130) + 3528             # the city block
GDT = hi(-32131) + 29952          # the time block
FAR_PTR = hi(-32120) - 14380      # the far-model drawable present
SNAP = hi(-32120) - 14444 + 1     # snap-LOD flag
DIR_FLAG = hi(-32120) - 27381
TB_FLAG = hi(-32120) - 27382
STREAM_B = hi(-32120) - 27376
PRESS = hi(-32120) - 14389
SKIP_F = hi(-32120) - 27384

COMMON = r'''
static uint32_t g_c[40];
static int nrec;
struct Rec{uint32_t r5,r6;uint32_t v[4];};
static Rec recs[8];
static uint32_t g_word;
'''

MAIN_RCH = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t T=0x40010000,DR=0x40010400,VP=0x40010800,MG=0x40010C00,RN=0x40011000;
 while(in>>k){rt_reset();nrec=0;g_c[0]=g_c[1]=0;
  if(k=="RCH"){
   uint32_t maxrots,rad,scale,low,fov,night,f432,f433,f373,tls,pass;
   in>>std::dec>>maxrots>>std::hex>>rad>>scale>>std::dec>>low>>std::hex>>fov>>night>>std::dec>>f432>>f433>>f373>>tls>>std::hex>>pass>>std::dec;
   for(uint32_t a=T;a<T+0x400;a+=4)store(a,0xDEADBEEFu);
   store(T+144,maxrots);store(T+168,DR);store(DR+112,rad);store(T+160,scale);*pointer(T+218)=low;store16(T+184,5);store(T+180,0);store(T+188,0);
   store(G_VP_,VP);store(VP+696,fov);store(G_MGR_,MG);store(MG+32,night);store(G_RN_,RN);store(G_CUR_,0);
   for(uint32_t a=RN;a<RN+0x200;a++)*pointer(a)=0;
   *pointer(RN+432)=f432;*pointer(RN+433)=f433;*pointer(RN+373)=f373;*pointer(load(RT_THREAD_)+72)=tls;
   Context cx=fresh();cx.r3.u32=T;cx.r4.u32=pass;sub_RCH(cx,nullptr);
   printf("%08x %08x %08x %d",load(T+132),load(T+136),load(T+140),nrec);
   for(int i=0;i<nrec;i++)printf(" [%u %u %08x %08x %08x %08x]",recs[i].r5,recs[i].r6,recs[i].v[0],recs[i].v[1],recs[i].v[2],recs[i].v[3]);
   printf(" %d %u %u\n",load(G_CUR_)==T,g_c[0],g_c[1]);
  } else if(k=="FAD"){
   uint32_t maxrots,nr,fr,scale,r5,r6;in>>std::dec>>maxrots>>std::hex>>nr>>fr>>scale>>std::dec>>r5>>r6;
   for(uint32_t a=T;a<T+0x400;a+=4)store(a,0);
   store(T+144,maxrots);store(T+132,nr);store(T+136,fr);
   Context cx=fresh();cx.r3.u32=T;cx.r5.u32=r5;cx.r6.u32=r6;{uint32_t b=scale;float f;std::memcpy(&f,&b,4);cx.f1.f64=f;}
   sub_FAD(cx,nullptr);float rf=float(cx.f1.f64);uint32_t rb;std::memcpy(&rb,&rf,4);
   printf("%08x %08x %08x %08x %08x\n",recs[0].v[0],recs[0].v[1],recs[0].v[2],recs[0].v[3],rb);
  } else if(k=="PAU"){
   uint32_t r4,set,f373,tls;in>>std::dec>>r4>>set>>f373>>tls;
   for(uint32_t a=RN;a<RN+0x200;a++)*pointer(a)=0;*pointer(RN+373)=f373;*pointer(load(RT_THREAD_)+72)=tls;
   if(set){Context c=fresh();c.r3.u32=RN;c.r4.u32=r4;sub_SET(c,nullptr);}
   Context c=fresh();c.r3.u32=RN;sub_GET(c,nullptr);
   printf("%u\n",c.r3.u32&0xFF);
  }
 }
}
'''

MAIN_OCC = r'''
static void setv(Context&cx,int which,const uint32_t w[4],uint32_t at){for(int i=0;i<4;i++)store(at+4*i,w[i]);
 simde__m128i m=simde_mm_shuffle_epi8(simde_mm_load_si128((simde__m128i*)pointer(at)),simde_mm_load_si128((simde__m128i*)VectorMaskL));
 if(which==1)simde_mm_store_si128((simde__m128i*)cx.v1.u8,m);else simde_mm_store_si128((simde__m128i*)cx.v63.u8,m);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 while(in>>k){rt_reset();nrec=0;
  if(k=="OCCA"){
   uint32_t w[4],word;in>>std::hex>>word>>w[0]>>w[1]>>w[2]>>w[3];g_word=word;
   store(G_OCC_,0x40010000u);store(G_VIEW_,3);
   Context cx=fresh();setv(cx,1,w,0x40010100u);sub_OA(cx,nullptr);
   printf("%u %u %08x %08x %08x %08x %u\n",cx.r3.u32&0xFF,recs[0].r6,recs[0].v[0],recs[0].v[1],recs[0].v[2],recs[0].v[3],recs[0].r5);
  } else if(k=="OCCB"){
   uint32_t word,flags,w[4];in>>std::hex>>word>>std::dec>>flags>>std::hex>>w[0]>>w[1]>>w[2]>>w[3];g_word=word;
   Context cx=fresh();cx.r3.u32=0x40010000u;cx.r4.u32=7;cx.r7.u32=flags;
   {uint32_t b=w[3];float f;std::memcpy(&f,&b,4);cx.f1.f64=f;}
   setv(cx,63,w,0x40010100u);cx.r5.u32=0x40010100u;sub_OB(cx,nullptr);
   printf("%08x %u %08x %08x %08x %08x %u\n",cx.r3.u32,recs[0].r6,recs[0].v[0],recs[0].v[1],recs[0].v[2],recs[0].v[3],recs[0].r5);
  }
 }
}
'''

MAIN_LNS = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t RN=0x40011000,VP=0x40010800,DIRO=0x40012000;
 while(in>>k){rt_reset();
  uint32_t fov,sc,narrow,rate,dt,base,f432,f433,f373,tls;in>>std::hex>>fov>>sc>>narrow>>rate>>dt>>base>>std::dec>>f432>>f433>>f373>>tls;
  for(uint32_t a=RN;a<RN+0x200;a++)*pointer(a)=0;
  store(D_,DIRO);
  *pointer(DIR_FLAG_)=0;*pointer(TB_FLAG_)=0;
  store(D_+36,sc);store(D_+40,narrow);store(D_+24,base);
  store(D_+3572-3528,rate);
  store(G_RN_,RN);store(VIEW_IDX_,0);store(RN+(0+45)*4,VP);store(VP+696,fov);
  store(GDT_+8,dt);
  *pointer(RN+432)=f432;*pointer(RN+433)=f433;*pointer(RN+373)=f373;*pointer(load(RT_THREAD_)+72)=tls;
  store(STREAM_,1);*pointer(PRESS_)=0;*pointer(SKIPF_)=1;store(D_+32,0);
  Context cx=fresh();sub_LNS(cx,nullptr);
  printf("%08x %08x\n",load(D_+36),load(D_+3664-3528));
 }
}
'''

MAIN_FDE = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;const uint32_t R=0x40010000;
 while(in>>k){rt_reset();
  uint32_t st,dist,thr,t,fl,fs,ch,dt,cross,det,snap,far;in>>std::dec>>st>>std::hex>>dist>>thr>>t>>std::dec>>fl>>fs>>ch>>std::hex>>dt>>std::dec>>cross>>det>>snap>>far;
  for(uint32_t a=R;a<R+0x400;a++)*pointer(a)=0;
  store(R+252,st);store(R+268,dist);store(R+288,t);store16(R+292,fl);*pointer(R+295)=fs;*pointer(R+296)=ch;
  store(D_+3664-3528,thr);*pointer(D_+3676-3528)=det;*pointer(D_+3612-3528+10)=cross;*pointer(SNAP_)=snap;store(FARP_,far?0x40010800u:0);
  store(D_+3612-3528,0x40400000u);store(GDT_+88,dt);
  Context cx=fresh();cx.r3.u32=R;sub_FDE(cx,nullptr);
  printf("%u %08x %u\n",*pointer(R+295),load(R+288),*pointer(R+296));
 }
}
'''


def fill(main, **kw):
    main = main.replace('RT_THREAD_', '0x%08Xu' % rd.RT_THREAD)
    for k, v in kw.items():
        main = re.sub(r'\b' + k + r'\b', '0x%08Xu' % v, main)
    return main


def run_exe(exe, image, cases):
    p = subprocess.run([str(exe), str(image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    return p.stdout.splitlines()


def f32_next(x):
    return struct.unpack('>f', struct.pack('>I', fb(x) + 1))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--seed', type=int, default=63362)
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    a.work.mkdir(parents=True, exist_ok=True)

    def name(fid):
        return 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    rnd = random.Random(a.seed)
    out = []

    def finish(tag, cases, outs):
        assert len(cases) == len(outs), (tag, len(cases), len(outs))
        for c, o in zip(cases, outs):
            out.append(f'{c} -> {o}')

    def go(exe, tag, cases):
        cf = a.work / f'{tag}_cases.txt'
        cf.write_text('\n'.join(cases) + '\n')
        finish(tag, cases, run_exe(exe, a.image, cf))

    rc.EXTRA += COMMON + 'union PPCRegister;void mcla_city_lod(PPCRegister&);' + chr(10)
    # ---- RCH / FAD / PAU --------------------------------------------------------------------------------------------------
    setter = 'if(nrec<8){Rec&r=recs[nrec];r.r5=ctx.r5.u32;r.r6=ctx.r6.u32;for(int i=0;i<4;i++)r.v[i]=load(ctx.r4.u32+4*i);}nrec++;'
    stubs = {name('F-32053'): setter, name('F-93617'): 'g_c[0]++;', name('F-48104'): 'ctx.r3.u32=0;g_c[1]++;',
             name('F-84808'): '', name('F-42781'): ''}
    src = (fill(MAIN_RCH, G_VP_=G_VP, G_MGR_=G_MGR, G_RN_=G_RN, G_CUR_=G_CUR).replace('sub_RCH(', name('F-63362') + '(')
           .replace('sub_FAD(', name('F-98063') + '(').replace('sub_SET(', name('F-65267') + '(').replace('sub_GET(', name('F-31789') + '('))
    exe = rd.build(idx, [name('F-63362'), name('F-98063'), name('F-65267'), name('F-31789')], src, a.work, 'map_reach_rch', image=img, stubs=stubs)
    cases = []
    for _ in range(6000):
        maxrots = rnd.choice([0, 1, 1, 1, 2, 250, 250, 3])
        radius = rnd.choice([rnd.uniform(0.05, 3.0), rnd.uniform(0.5, 12.0), rnd.uniform(1.0, 60.0), 1.0, 2.5])
        scale = rnd.choice([rnd.uniform(0.2, 2.0), rnd.uniform(0.5, 5.0), 1.0, 1.0, 2.0, 2.0000002, 1.9999999, 0.0])
        low = rnd.choice([0, 0, 0, 1, 255])
        fov = rnd.choice([25.0, 25.0, 30.0, 37.5, 50.0, 60.0, 75.0, 90.0, rnd.uniform(15, 120), rnd.uniform(24, 60), rnd.uniform(24, 60)])
        night = rnd.choice([1.0, 1.0, 2.0, 2.0, rnd.uniform(1, 2), rnd.uniform(1, 2), 1.5])
        f432, f433 = rnd.choice([0, 1]), rnd.choice([0, 1, 1, 0, 2, 255])
        f373, tls = rnd.choice([0, 1, 1, 0]), rnd.choice([0, 0, 0, 1])
        pass_ = rnd.choice([0x80000, 0x800, 0x80000, 0x800, 0x40])
        cases.append(f'RCH {maxrots} {fb(radius):08x} {fb(scale):08x} {low} {fb(fov):08x} {fb(night):08x} {f432} {f433} {f373} {tls} {pass_:x}')
    go(exe, 'RCH', cases)
    cases = []
    for _ in range(3000):
        maxrots = rnd.choice([0, 1, 2, 250])
        nr = rnd.choice([rnd.uniform(-10000, -9999), rnd.uniform(0, 3000), rnd.uniform(1, 100), 0.0])
        fr = rnd.choice([rnd.uniform(0, 6000), rnd.uniform(1, 400)])
        scale = rnd.choice([rnd.uniform(0.2, 2.0), rnd.uniform(0.5, 5.0), 1.0, 2.0, 0.0])
        cases.append(f'FAD {maxrots} {fb(nr):08x} {fb(fr):08x} {fb(scale):08x} {rnd.choice([0, 1])} {rnd.choice([0, 1])}')
    go(exe, 'FAD', cases)
    go(exe, 'PAU', [f'PAU {r4} {s} {f373} {tls}' for r4 in (0, 1, 2, 255, 256) for s in (0, 1) for f373 in (0, 1) for tls in (0, 1)])

    # ---- OCC --------------------------------------------------------------------------------------------------------------
    occ_stub = {name('F-38575'): 'if(nrec<8){Rec&r=recs[nrec];r.r5=ctx.r5.u32==0x40010100u;r.r6=ctx.r6.u32;for(int i=0;i<4;i++)r.v[i]=load(ctx.r5.u32+4*i);}nrec++;ctx.r3.u32=g_word;'}
    src = fill(MAIN_OCC, G_OCC_=G_OCC, G_VIEW_=G_VIEW).replace('sub_OA(', name('F-71608') + '(').replace('sub_OB(', name('F-87100') + '(')
    exe = rd.build(idx, [name('F-71608'), name('F-87100')], src, a.work, 'map_reach_occ', image=img, stubs=occ_stub)
    words = [0, 0x04000000, 0x10000000, 0x14000000, 0x08000000, 0x20000000, 0x30000000, 0x28000000, 0x2C000000, 0x18000000, 0x1C000000,
             0xFFFFFFFF, 0xFBFFFFFF, 0xEFFFFFFF, 0xEBFFFFFF, 0x00000001, 0x80000000, 0x3FFFFFFF, 0x01000000, 0x02000000, 0x00FFFFFF] \
        + [rnd.getrandbits(32) for _ in range(300)]

    def sphere():
        return [fb(rnd.uniform(-3000, 3000)), fb(rnd.uniform(-50, 300)), fb(rnd.uniform(-3000, 3000)), fb(rnd.uniform(0.1, 20))]
    cases = [f'OCCA {w:08x} ' + ' '.join(f'{x:08x}' for x in sphere()) for w in words]
    cases += [f'OCCB {w:08x} {rnd.choice([0, 1, 2, 3, 7, 0x20, 0x7FFFFF])} ' + ' '.join(f'{x:08x}' for x in sphere()) for w in words]
    go(exe, 'OCC', cases)

    # ---- LNS --------------------------------------------------------------------------------------------------------------
    lns_stubs = {name(f): '' for f in ('F-50459', 'F-74970', 'F-66201', 'F-51336', 'F-88687', 'F-89368', 'F-72386', 'F-62577', 'F-38010')}
    lns_stubs[name('F-50459')] = 'ctx.r3.u32=0;'
    src = fill(MAIN_LNS, D_=D, DIR_FLAG_=DIR_FLAG, TB_FLAG_=TB_FLAG, G_RN_=G_RN, VIEW_IDX_=G_VIEW, GDT_=GDT, STREAM_=STREAM_B,
               PRESS_=PRESS, SKIPF_=SKIP_F).replace('sub_LNS(', name('F-50083') + '(')
    exe = rd.build(idx, [name('F-50083')], 'void mcla_city_lod(PPCRegister&f13){}\n' + src, a.work, 'map_reach_lns', image=img, stubs=lns_stubs)
    cases = []
    for _ in range(4000):
        fov = rnd.choice([25.0, 25.0, 30.0, 37.5, 50.0, 49.99, 50.01, 60.0, 75.0, 90.0, 20.0, rnd.uniform(10, 120), rnd.uniform(24, 52), rnd.uniform(24, 52)])
        sc = rnd.choice([1.5, 1.0, rnd.uniform(1.0, 1.5), rnd.uniform(0.2, 3.0), rnd.uniform(1.0, 1.5), 1.2345678, 0.0, 100.0])
        narrow, rate = rnd.choice([1.5, 1.5, 1.5, 1.25]), rnd.choice([0.5, 0.5, 0.5, 2.0])
        dt = rnd.choice([1 / 60, 1 / 30, 0.0, rnd.uniform(0.0001, 0.1), rnd.uniform(0.0001, 0.1), 0.1])
        base = rnd.choice([300.0, 300.0, 300.0, 450.0, rnd.uniform(100, 600)])
        cases.append(f'LNS {fb(fov):08x} {fb(sc):08x} {fb(narrow):08x} {fb(rate):08x} {fb(dt):08x} {fb(base):08x} 0 0 0 0')
    for _ in range(400):   # the paused flag set (the original doubles the scale and stores it back)
        fov = rnd.choice([25.0, 40.0, 60.0, rnd.uniform(20, 70)])
        sc = rnd.choice([1.5, 1.0, rnd.uniform(1.0, 3.0), 0.7])
        flags = rnd.choice([(1, 1, 0, 0), (1, 1, 0, 0), (0, 0, 1, 0), (1, 0, 1, 0), (1, 1, 1, 1), (1, 0, 0, 0)])
        cases.append(f'LNS {fb(fov):08x} {fb(sc):08x} {fb(1.5):08x} {fb(0.5):08x} {fb(rnd.choice([1 / 60, 1 / 30])):08x} {fb(300.0):08x} {flags[0]} {flags[1]} {flags[2]} {flags[3]}')
    go(exe, 'LNS', cases)

    # ---- FDE --------------------------------------------------------------------------------------------------------------
    src = fill(MAIN_FDE, D_=D, GDT_=GDT, SNAP_=SNAP, FARP_=FAR_PTR).replace('sub_FDE(', name('F-89177') + '(')
    exe = rd.build(idx, [name('F-89177')], src, a.work, 'map_reach_fde', image=img)
    cases = []
    ts = [0.0, 0.0, 0.25, 0.5, 0.6666667, 0.9, 0.99, 1.0, 0.9999999, 1.1, 0.3333333]
    for _ in range(12000):
        st = rnd.choice([3, 3, 3, 3, 3, 3, 0, 1, 2, 4, 5])
        thr = rnd.choice([450.0, 300.0, 300.0, rnd.uniform(0, 900)])
        dist = rnd.choice([thr, 0.0, rnd.uniform(0, 900), thr * 0.999, thr * 1.001, f32_next(thr), 1e9, float('inf')])
        t = rnd.choice(ts + [rnd.uniform(0, 1.2)])
        fl = rnd.choice([0, 1, 2, 3, 2, 2, 2, 0x10, 0x12, 0xFFFF])
        fs = rnd.choice([1, 1, 1, 2, 4, 2, 4, 0, 3, 0xFF, 0x81])
        ch = rnd.choice([0, 1, 2, 4, 5, 6, 7, 6, 5, 6, 5, 3, 8, 0x80, 0xFF])
        dt = rnd.choice([1 / 60, 1 / 30, 0.0, rnd.uniform(0.0001, 0.1), rnd.uniform(0.0001, 0.1), 0.1, 0.34, 0.3333333, 0.1111111, 0.05])
        shipped = rnd.random() < 0.9
        cross, det, snap, far = (1, 1, 0, 1) if shipped else (rnd.choice([0, 1]), rnd.choice([0, 1]), rnd.choice([0, 1]), rnd.choice([0, 1]))
        cases.append(f'FDE {st} {fb(dist):08x} {fb(thr):08x} {fb(t):08x} {fl} {fs} {ch} {fb(dt):08x} {cross} {det} {snap} {far}')
    go(exe, 'FDE', cases)

    keep = [l for l in out if not re.search(r'(?<![0-9a-zA-Z])82[0-9a-f]{6}(?![0-9a-zA-Z])', l)]
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(keep), 'of', len(out), 'cases')


if __name__ == '__main__':
    main()
