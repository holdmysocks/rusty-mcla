"""The original's MISSION RESULTS selection and its "updated" bytes run offline (recomp_dispatch): unchanged generated code on a flat
guest memory. Writes ui_mission_update_ref.txt, one case per line, numbers in hex:
  L n (state hidden should updated b684 req140 cond84) x n -> collected indices ; +690 / +140 / +84 of each after F-75010
      a mission manager of n missions (state objects id N, mask 1 << N as F-33894 makes them), the collector F-65871 with mask 60,
      filter kind 4, district 6 (F-78851); then the form's hide F-75010 (-> F-92621 -> F-19595) over the manager
  U msg_updated cond_in_req satisfied allow mission_msg member -> req140 mission690
      a requirement inside a mission's objectives (or a foreign one), F-64780 handed a message (its +108 handle or another) from a
      condition (one of its +124 list or another); a post of Requirement_Updated (F-41407, recorded) is handed to the mission's
      listener F-30196 with its +660 handle or another
Usage: ui_mission_update_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
POST = 0x40000F00


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 char kind[4];
 while(scanf("%3s",kind)==1){
  rt_reset();
  if(kind[0]=='L'){
   uint32_t n;scanf("%x",&n);
   uint32_t mgr=rt_alloc(64,16),list=rt_alloc(4*n+4,16),out=rt_alloc(16,16);
   for(uint32_t i=0;i<64;i+=4)store(mgr+i,0);for(uint32_t i=0;i<16;i+=4)store(out+i,0);
   store(mgr+28,list);store(mgr+32,n<<16);
   std::vector<uint32_t> ms(n),rq(n),cd(n);
   for(uint32_t k=0;k<n;k++){
    uint32_t v[7];for(int j=0;j<7;j++)scanf("%x",&v[j]);
    uint32_t m=rt_alloc(768,16),st=rt_alloc(16,16),r=rt_alloc(160,16),c=rt_alloc(96,16),ca=rt_alloc(8,16);
    for(uint32_t i=0;i<768;i+=4)store(m+i,0);for(uint32_t i=0;i<160;i+=4)store(r+i,0);for(uint32_t i=0;i<96;i+=4)store(c+i,0);
    store(st,0);store(st+4,v[0]);store(st+8,1u<<v[0]);store(m+656,st);
    *pointer(m+685)=v[1];*pointer(m+691)=v[2];*pointer(m+690)=v[3];*pointer(m+684)=v[4];
    *pointer(m+456)='x';*pointer(m+457)=0;
    store(m+716,r);store(m+720,1u<<16);*pointer(r+140)=v[5];
    store(ca,c);store(r+124,ca);store(r+128,1u<<16);*pointer(c+84)=v[6];
    store(list+4*k,m);ms[k]=m;rq[k]=r;cd[k]=c;
   }
   Context c1=fresh();c1.r3.u32=mgr;c1.r4.u32=60;c1.r5.u32=4;c1.r6.u32=out;c1.r7.u32=6;sub_COLLECT(c1,nullptr);
   uint32_t cnt=load(out+4)>>16,arr=load(out);
   printf("OUT");
   for(uint32_t k=0;k<cnt;k++){uint32_t p=load(arr+4*k);for(uint32_t j=0;j<n;j++)if(ms[j]==p)printf(" %x",j);}
   printf(" ;");
   Context c2=fresh();c2.r3.u32=mgr;sub_CLEAR(c2,nullptr);
   for(uint32_t k=0;k<n;k++)printf(" %x %x %x",*pointer(ms[k]+690),*pointer(rq[k]+140),*pointer(cd[k]+84));
   printf("\n");
  }else{
   uint32_t v[6];for(int j=0;j<6;j++)scanf("%x",&v[j]);
   const uint32_t HU=0x1000,HC=0x2000,HM=0x4000,HMC=0x6000;
   uint32_t m=rt_alloc(768,16),objs=rt_alloc(2*144,16),foreign=rt_alloc(160,16),c=rt_alloc(96,16),c2=rt_alloc(96,16),ca=rt_alloc(8,16);
   for(uint32_t i=0;i<768;i+=4)store(m+i,0);for(uint32_t i=0;i<288;i+=4)store(objs+i,0);for(uint32_t i=0;i<160;i+=4)store(foreign+i,0);
   for(uint32_t i=0;i<96;i+=4){store(c+i,0);store(c2+i,0);}
   store(m+716,objs);store(m+720,2u<<16);store(m+660,HM);store(m+664,HMC);
   uint32_t r=v[5]?objs+144:foreign;
   store(r+108,HU);store(r+112,HC);*pointer(r+116)=v[2];*pointer(r+120)=v[3];store(ca,c);store(r+124,ca);store(r+128,1u<<16);
   store(0x40000F00u,0);
   Context a=fresh();a.r3.u32=r+4;a.r4.u32=v[0]?HU:0x3000;a.r5.u32=v[1]?c:c2;sub_REQ(a,nullptr);
   uint32_t sender=load(0x40000F00u);
   if(sender){Context b=fresh();b.r3.u32=m+4;b.r4.u32=v[4]?HM:0x5000;b.r5.u32=sender;sub_MIS(b,nullptr);}
   printf("OUT %x %x\n",*pointer(r+140),*pointer(m+690));
  }
  fflush(stdout);
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 600
    r = random.Random(30196)
    cases = []
    for _ in range(count):
        n = r.randrange(1, 9)
        ms = []
        for _ in range(n):
            ms += [r.randrange(6), r.choice([0, 0, 1, 2]), r.choice([0, 1, 1]), r.choice([0, 1, 1]), r.choice([0, 0x80, 0x7f]), r.randrange(2), r.randrange(2)]
        cases.append(['L', n] + ms)
    for bits in range(64):
        cases.append(['U'] + [(bits >> k) & 1 for k in range(6)])
    for _ in range(count // 2):
        cases.append(['U'] + [r.randrange(2) for _ in range(6)])
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_mupd'
    work.mkdir(exist_ok=True)
    f = {k: 'sub_' + addr(k) for k in ('F-65871', 'F-75010', 'F-64780', 'F-30196')}
    # the hub's post records the sender; the paths this harness does not take throw if reached
    stubs = {'sub_' + addr('F-41407'): 'store(0x40000F00u,ctx.r5.u32);'}
    # (the district name lookup: district 6 takes every mission whatever it returns)
    stubs['sub_' + addr('F-67263')] = 'ctx.r3.s32=-1;'
    for k in ('F-85221', 'F-32055'):
        stubs['sub_' + addr(k)] = 'throw std::runtime_error("condition complete path reached");'
    main_src = MAIN.replace('sub_COLLECT', f['F-65871']).replace('sub_CLEAR', f['F-75010']).replace('sub_REQ', f['F-64780']).replace('sub_MIS', f['F-30196'])
    exe = rd.build(idx, list(f.values()), main_src, work, 'mupd', image=img, stubs=stubs)
    inp = '\n'.join(' '.join(x if isinstance(x, str) else '%x' % x for x in c) for c in cases) + '\n'
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[3:].strip() for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%s -> %s\n' % (' '.join(x if isinstance(x, str) else '%x' % x for x in c), o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
