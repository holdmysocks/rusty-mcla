"""The GPS map's bank and experience panel run offline (recomp_dispatch): what the original's fill F-71884 (with the bank F-43310),
its constructor F-51730 and its cursor test F-57096 hand to the movie. Writes ui_gps_panel_ref.txt, one case per line, numbers in hex:
  P rank perf rep money lang vtype amount max next base index lo hi -> the movie's variables in the order set
      rank = F-62421's answer (0..12), perf = F-69503's (0..3 = A..D, 4 none), rep = the experience F-68013 +8, money = the manager's
      +508, lang = the string table's +28, vtype = the vehicle class index (F-45718 +24, table 0x827E9ABC), amount / max = the class
      system's +8 / +12, next / base / index = F-53104 / F-57870 / F-96803's answers, lo / hi = the manager's thresholds
      (rank + 38) x 4 and (min(rank + 1, 12) + 38) x 4 as float bits.
      Output: S:name=text (a string variable), F:name=float bits; the string table F-53775 / F-41524 / F-45191 / F-16600 is a stub
      that returns "{key}" ("$<D>Navmap_Bank_Amount</D>" for Navmap_Bank_Desc) and keeps the dynamic variables; the money text
      F-27560 -> F-36073 runs as is.
  V width scale x y -> I:panelisvisible=v and the panel's +128 +132 +144 +148 as float bits
      width / scale = the main viewport's +688 / +648, x y = the cursor NM+448 as F-25309 projects it (stubbed, float bits).
Usage: ui_gps_panel_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
CASE = 0x40000C00
MOVIE = 0x40000F00
VT468 = 0x7FFF0010    # the panel's vtable slot 117: its movie (a fake address)
GLOBALS = 0x7FFF0020  # never called: carries the host helpers below into the source


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


HELPERS = r'''}
static std::vector<std::pair<std::string,std::string>> VARS;
static std::string gstr(uint32_t a){std::string s;for(;a&&*pointer(a);a++)s+=char(*pointer(a));return s;}
static uint32_t gput(const std::string&s){uint32_t p=rt_alloc(uint32_t(s.size())+1,16);for(size_t i=0;i<=s.size();i++)*pointer(p+uint32_t(i))=i<s.size()?uint8_t(s[i]):0;return p;}
static std::string*var(const std::string&k){for(auto&p:VARS)if(p.first==k)return &p.second;return nullptr;}
static void setvar(const std::string&k,const std::string&v){if(auto*p=var(k))*p=v;else VARS.push_back({k,v});}
static std::string lookup(const std::string&k){if(auto*p=var(k))return *p;
 std::string v=k=="Navmap_Bank_Desc"?"$<D>Navmap_Bank_Amount</D>":"{"+k+"}";size_t a;
 while((a=v.find("<D>"))!=std::string::npos){size_t b=v.find("</D>",a);auto*p=var(v.substr(a+3,b-a-3));v=v.substr(0,a)+(p?*p:std::string())+v.substr(b+4);}
 return v;}
static void unused_helpers_(Context &ctx,const uint8_t *base){'''

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 char kind[4];
 while(scanf("%3s",kind)==1){
  rt_reset();VARS.clear();
  uint32_t po=rt_alloc(512,16),vt=rt_alloc(1024,16);
  for(uint32_t i=0;i<512;i+=4)store(po+i,0);store(vt+468,VT468_);
  printf("OUT");
  if(kind[0]=='P'){
   uint32_t rank,perf,rep,money,lang,vtype,amount,mx,nx,bs,ix,lo,hi;
   scanf("%x %x %x %x %x %x %x %x %x %x %x %x %x",&rank,&perf,&rep,&money,&lang,&vtype,&amount,&mx,&nx,&bs,&ix,&lo,&hi);
   uint32_t mgr=rt_alloc(1024,16),stt=rt_alloc(64,16),pl=rt_alloc(512,16);
   for(uint32_t i=0;i<1024;i+=4)store(mgr+i,0);for(uint32_t i=0;i<64;i+=4)store(stt+i,0);for(uint32_t i=0;i<512;i+=4)store(pl+i,0);
   store(0x8287484Cu,mgr);store(0x8286D7FCu,stt);store(0x82874374u,pl);store(pl+132,1);store(pl+8,1);
   store(mgr+508,money);store(stt+28,lang);
   uint32_t up=rank+1<12?rank+1:12;store(mgr+(rank+38)*4,lo);store(mgr+(up+38)*4,hi);
   store(CASE_+0,rank);store(CASE_+4,perf);store(CASE_+8,nx);store(CASE_+12,bs);store(CASE_+16,ix);store(CASE_+0x28,rep);
   store(CASE_+0x58,vtype);store(CASE_+0x88,amount);store(CASE_+0x8C,mx);
   store(po,vt);
   Context c=fresh();c.r3.u32=po;sub_FILL(c,nullptr);
  }else{
   uint32_t w,sc,x,y;scanf("%x %x %x %x",&w,&sc,&x,&y);
   uint32_t vp=rt_alloc(1024,16),tab=rt_alloc(256,16),nm=rt_alloc(1024,16);
   for(uint32_t i=0;i<1024;i+=4)store(vp+i,0);
   store(0x8288B9F0u,0);store(0x8287E064u,tab);store(tab+45*4,vp);store(vp+648,sc);store(vp+688,w);store(0x8286D814u,nm);
   store(CASE_+0x30,x);store(CASE_+0x34,y);
   {Context c=fresh();c.r3.u32=po;sub_CTOR(c,nullptr);}
   store(po,vt);
   {Context c=fresh();c.r3.u32=po;sub_VIS(c,nullptr);}
   printf("\tR:%x %x %x %x",load(po+128),load(po+132),load(po+144),load(po+148));
  }
  printf("\n");fflush(stdout);
 }
}
'''


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 600
    r = random.Random(71884)
    cases = []
    for k in range(count):
        if k % 4 == 3:
            w = r.choice([640, 960, 999, 1000, 1280, 1920])
            sc = r.choice([1.0, 0.75, 0.5, 1.5, r.uniform(0.3, 2.0)])
            x = r.choice([r.uniform(-200, 1500), 420.0, 419.99, 220.0, 219.99, r.uniform(0, 500)])
            y = r.choice([r.uniform(-200, 900), 270.0, 269.99, 170.0, 169.99, r.uniform(0, 300)])
            cases.append('V %x %x %x %x' % (w, fbits(sc), fbits(x), fbits(y)))
            continue
        rank = r.randrange(13)
        lo = r.choice([0.0, r.uniform(0, 1e6), float(r.randrange(0, 200000))])
        hi = lo + r.choice([0.0, 0.5, r.uniform(1, 1e5), float(r.randrange(1, 50000))])
        rep = max(0, int(lo + r.uniform(-10, (hi - lo) + 10)))
        mx = r.randrange(1, 5000)
        bs = r.randrange(0, mx)
        nx = r.choice([bs + r.randrange(1, 3000), bs])
        cases.append('P %x %x %x %x %x %x %x %x %x %x %x %x %x' % (
            rank, r.randrange(5), rep, r.choice([0, 7, 999, 1000, 123456, r.randrange(0, 2140000000)]), r.randrange(5),
            r.randrange(10), r.randrange(0, 6000), mx, nx, bs, r.randrange(0, 12), fbits(lo), fbits(hi)))
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_gpspanel'
    work.mkdir(exist_ok=True)
    f = {k: 'sub_' + addr(k) for k in ('F-71884', 'F-51730', 'F-57096')}
    st = lambda k: 'sub_' + addr(k)
    stubs = {
        st('F-53775'): 'ctx.r3.u32=gput(lookup(gstr(ctx.r4.u32)));',
        st('F-41524'): 'setvar(gstr(ctx.r4.u32),gstr(ctx.r5.u32));',
        st('F-45191'): 'VaIn v{&ctx,1,0,0};setvar(gstr(ctx.r4.u32),fmt_c(gstr(ctx.r5.u32),v));',
        st('F-16600'): 'printf("\\tS:%s=%s",gstr(ctx.r4.u32).c_str(),gstr(ctx.r5.u32).c_str());',
        st('F-66095'): '{float x=float(ctx.f1.f64);uint32_t b;memcpy(&b,&x,4);printf("\\tF:%s=%x",gstr(ctx.r4.u32).c_str(),b);}',
        st('F-59146'): 'printf("\\tI:%s=%d",gstr(ctx.r4.u32).c_str(),int(ctx.r5.u32));',
        st('F-63696'): '', st('F-96040'): '', st('F-96685'): '',
        st('F-55479'): 'ctx.r3.u32=CASE_+0x100;',
        st('F-68013'): 'ctx.r3.u32=CASE_+0x20;',
        st('F-62421'): 'ctx.r3.u32=load(CASE_+0);',
        st('F-45718'): 'ctx.r3.u32=CASE_+0x40;',
        st('F-53701'): 'ctx.r3.u32=0;',
        st('F-86078'): 'ctx.r3.u32=CASE_+0x80;',
        st('F-69503'): 'ctx.r3.u32=load(CASE_+4);',
        st('F-15013'): 'ctx.r3.u32=gput("VEHNAME");',
        st('F-53104'): 'ctx.r3.u32=load(CASE_+8);',
        st('F-57870'): 'ctx.r3.u32=load(CASE_+12);',
        st('F-96803'): 'ctx.r3.u32=load(CASE_+16);',
        st('F-25309'): 'store(ctx.r5.u32,load(CASE_+0x30));store(ctx.r6.u32,load(CASE_+0x34));',
    }
    stubs = {k: v.replace('CASE_', '0x%Xu' % CASE) for k, v in stubs.items()}
    main_src = (MAIN.replace('CASE_', '0x%Xu' % CASE).replace('VT468_', '0x%Xu' % VT468).replace('sub_FILL', f['F-71884'])
                .replace('sub_CTOR', f['F-51730']).replace('sub_VIS', f['F-57096']))
    exe = rd.build(idx, list(f.values()), main_src, work, 'gpspanel', image=img, stubs=stubs,
                   fakes={GLOBALS: HELPERS, VT468: 'ctx.r3.u32=0x%Xu;' % MOVIE})
    inp = '\n'.join(cases) + '\n'
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[3:].strip('\n') for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%s ->%s\n' % (c, o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
