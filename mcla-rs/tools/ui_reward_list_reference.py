"""The original's pending reward list run offline (recomp_dispatch): unchanged generated code on a flat guest memory.
Writes ui_reward_list_ref.txt, one case per line, numbers in hex:
  R n (hidden give_now) x n ; op .. -> result ..
      n rewards (+8 bHidden, +9 bGiveImmediately, a vtable whose slot 2 - the give - is recorded); the reward manager (its pointer at
      the global the REWARDS form reads) starts empty; each op is
        a<k>  AddReward F-64223 of reward k            -> the rewards given by the call (`.` none)
        g     the REWARDS getter F-22948               -> 2 or 3
        p     the page init's give-all F-73125         -> the rewards given, in order
        h     the page hide's clear F-77851            -> `-`
      and last the list's contents.
Usage: ui_reward_list_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
GIVE = 0x7E000010
GLOBAL = 0x82874848


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
static void given(uint32_t mgr,uint32_t*rw,uint32_t n){uint32_t c=load(mgr+64);if(!c){printf(" .");return;}printf(" ");
 for(uint32_t i=0;i<c;i++){uint32_t r=load(mgr+68+4*i);for(uint32_t k=0;k<n;k++)if(rw[k]==r)printf("%x",k);}store(mgr+64,0);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 char kind[4];
 while(scanf("%3s",kind)==1){
  rt_reset();
  uint32_t n;scanf("%x",&n);
  uint32_t mgr=rt_alloc(256,16),vt=rt_alloc(16,16),rw[16];
  for(uint32_t i=0;i<256;i+=4)store(mgr+i,0);
  store(vt,0);store(vt+4,0);store(vt+8,GIVE_ADDR);store(vt+12,0);
  store(GLOBAL_ADDR,mgr);
  for(uint32_t k=0;k<n;k++){uint32_t h,g;scanf("%x %x",&h,&g);rw[k]=rt_alloc(16,16);store(rw[k],vt);store(rw[k]+4,0);store(rw[k]+8,0);store(rw[k]+12,0);
   *pointer(rw[k]+8)=h;*pointer(rw[k]+9)=g;}
  char op[8];printf("OUT");
  while(scanf("%7s",op)==1&&op[0]!=';'){
   if(op[0]=='a'){uint32_t k=strtoul(op+1,0,16);Context c=fresh();c.r3.u32=mgr;c.r4.u32=rw[k];sub_ADD(c,nullptr);given(mgr,rw,n);}
   else if(op[0]=='g'){Context c=fresh();c.r3.u32=0;sub_GET(c,nullptr);printf(" %x",c.r3.u32);}
   else if(op[0]=='p'){Context c=fresh();c.r3.u32=load(GLOBAL_ADDR);sub_GIVE(c,nullptr);given(mgr,rw,n);}
   else if(op[0]=='h'){Context c=fresh();c.r3.u32=load(GLOBAL_ADDR);sub_CLEAR(c,nullptr);printf(" -");}
  }
  uint32_t cnt=load(mgr+4)>>16,arr=load(mgr);printf(" |");
  for(uint32_t i=0;i<cnt;i++){uint32_t r=load(arr+4*i);for(uint32_t k=0;k<n;k++)if(rw[k]==r)printf(" %x",k);}
  printf("\n");fflush(stdout);
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 600
    r = random.Random(64223)
    cases = []
    for _ in range(count):
        n = r.randrange(1, 7)
        rw = [(r.randrange(2), r.randrange(2)) for _ in range(n)]
        ops = []
        for _ in range(r.randrange(1, 16)):
            o = r.choice('aaaaagph')
            ops.append('a%x' % r.randrange(n) if o == 'a' else o)
        cases.append((n, rw, ops))
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_rlist'
    work.mkdir(exist_ok=True)
    f = {k: 'sub_' + addr(k) for k in ('F-64223', 'F-22948', 'F-73125', 'F-77851')}
    main_src = (MAIN.replace('sub_ADD', f['F-64223']).replace('sub_GET', f['F-22948']).replace('sub_GIVE', f['F-73125'])
                .replace('sub_CLEAR', f['F-77851']).replace('GIVE_ADDR', '0x%Xu' % GIVE).replace('GLOBAL_ADDR', '0x%Xu' % GLOBAL))
    # the reward's give (vtable slot 2): recorded in the manager's spare words (+64 count, +68.. the rewards)
    fakes = {GIVE: 'uint32_t m=load(0x%Xu),c=load(m+64);store(m+68+4*c,ctx.r3.u32);store(m+64,c+1);' % GLOBAL}
    exe = rd.build(idx, list(f.values()), main_src, work, 'rlist', image=img, fakes=fakes)
    line = lambda c: 'R %x %s ; %s' % (c[0], ' '.join('%x %x' % x for x in c[1]), ' '.join(c[2]))
    inp = '\n'.join('R %x %s %s ;' % (c[0], ' '.join('%x %x' % x for x in c[1]), ' '.join(c[2])) for c in cases) + '\n'
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[3:].strip() for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%s -> %s\n' % (line(c), o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
