"""The original's threshold walk inside the pending reward list, run offline (recomp_dispatch): unchanged generated code on a flat guest memory.
Writes ui_reward_threshold_ref.txt, one case per line, numbers in hex:
  T n (hidden give_now sys)xn ; s (share m (pct r (reward)xr)xm)xs ; op .. -> given .. | reached ..
      n rewards (+8 bHidden, +9 bGiveImmediately, a vtable whose slot 2 - the give - is recorded); a reward with sys > 0 is an
      experience reward of system sys-1: its give sets that system's share and walks its thresholds (F-61830), as AddExperience
      does after the amount is added. s systems, each with its share after the experience and m thresholds (Percent, the rewards,
      reached clear, DownloadableBit 1). The share F-14176 is stubbed to the system's share. Each op is
        a<k>  AddReward F-64223 of reward k
        w<s>  the walk F-61830 of system s (a class point, added at once)
        p     the REWARDS page's give-all F-73125
      The output is every reward given, in order, then the reached flags of all thresholds.
Usage: ui_reward_threshold_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
GIVE = 0x7E000010
GLOBAL = 0x82874848
MASK = 0x827E9174


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
static uint32_t fbits(float f){uint32_t b;memcpy(&b,&f,4);return b;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 char kind[4];
 while(scanf("%3s",kind)==1){
  rt_reset();
  uint32_t n;scanf("%x",&n);
  uint32_t mgr=rt_alloc(1024,16),vt=rt_alloc(16,16),rw[32],sy[8],th[8][16],tm[8];
  for(uint32_t i=0;i<1024;i+=4)store(mgr+i,0);
  store(vt,0);store(vt+4,0);store(vt+8,GIVE_ADDR);store(vt+12,0);
  store(GLOBAL_ADDR,mgr);store(MASK_ADDR,1);
  for(uint32_t k=0;k<n;k++){uint32_t h,g,s;scanf("%x %x %x",&h,&g,&s);rw[k]=rt_alloc(32,16);for(int i=0;i<32;i+=4)store(rw[k]+i,0);store(rw[k],vt);
   *pointer(rw[k]+8)=h;*pointer(rw[k]+9)=g;store(rw[k]+20,k);}
  char sep[4];scanf("%3s",sep);
  uint32_t ns;scanf("%x",&ns);
  for(uint32_t s=0;s<ns;s++){float share;uint32_t m;scanf("%f %x",&share,&m);tm[s]=m;
   sy[s]=rt_alloc(256,16);for(int i=0;i<256;i+=4)store(sy[s]+i,0);
   uint32_t arr=rt_alloc(4*16+16,16);store(sy[s]+16,arr);store(sy[s]+20,m<<16);store(sy[s]+200,fbits(share));
   for(uint32_t t=0;t<m;t++){float pct;uint32_t r;scanf("%f %x",&pct,&r);
    uint32_t o=rt_alloc(128,16);for(int i=0;i<128;i+=4)store(o+i,0);th[s][t]=o;store(arr+4*t,o);
    uint32_t ra=rt_alloc(4*16+16,16);store(o+68,fbits(pct));store(o+72,ra);store(o+76,r<<16);store(o+84,1);
    for(uint32_t q=0;q<r;q++){uint32_t k;scanf("%x",&k);store(ra+4*q,rw[k]);}}}
  // the experience rewards: their system (+12)
  for(uint32_t k=0;k<n;k++){}
  store(mgr+1000,0);
  char op[8];printf("OUT");
  while(scanf("%7s",op)==1&&op[0]!=';'){
   if(op[0]=='a'){uint32_t k=strtoul(op+1,0,16);Context c=fresh();c.r3.u32=mgr;c.r4.u32=rw[k];sub_ADD(c,nullptr);}
   else if(op[0]=='w'){uint32_t s=strtoul(op+1,0,16);Context c=fresh();c.r3.u32=sy[s];c.r4.u32=0;sub_WALK(c,nullptr);}
   else if(op[0]=='p'){Context c=fresh();c.r3.u32=load(GLOBAL_ADDR);sub_GIVE(c,nullptr);}
  }
  uint32_t c=load(mgr+64);
  for(uint32_t i=0;i<c;i++)printf(" %x",load(load(mgr+68+4*i)+20));
  printf(" |");
  for(uint32_t s=0;s<ns;s++)for(uint32_t t=0;t<tm[s];t++)printf(" %x",(uint32_t)*pointer(th[s][t]+80));
  printf("\n");fflush(stdout);
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 600
    r = random.Random(61830)
    cases = []
    for _ in range(count):
        ns = r.randrange(1, 4)
        n = r.randrange(ns + 1, 13)
        # rewards 0..ns-1 are the experience rewards of systems 0..ns-1 (sys = s + 1); the rest plain
        rw = [(r.randrange(2), r.randrange(2), k + 1 if k < ns else 0) for k in range(n)]
        plain = list(range(ns, n))
        systems = []
        for s in range(ns):
            m = r.randrange(0, 6)
            share = r.choice([0.0, 0.05, 0.1, 0.2, 0.35, 0.5, 0.8, 1.0])
            ths = []
            for _ in range(m):
                pct = r.choice([0.02, 0.05, 0.1, 0.2, 0.35, 0.5, 0.6, 1.0])
                ths.append((pct, [r.choice(plain) for _ in range(r.randrange(0, 4))]))
            systems.append((share, ths))
        ops = []
        for _ in range(r.randrange(1, 12)):
            o = r.choice('aaaaawp')
            ops.append('a%x' % r.randrange(n) if o == 'a' else 'w%x' % r.randrange(ns) if o == 'w' else 'p')
        if r.randrange(2):
            ops.append('p')
        cases.append((rw, systems, ops))
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_rthr'
    work.mkdir(exist_ok=True)
    f = {k: 'sub_' + addr(k) for k in ('F-64223', 'F-73125', 'F-61830')}
    share = 'sub_' + addr('F-14176')
    main_src = (MAIN.replace('sub_ADD', f['F-64223']).replace('sub_GIVE', f['F-73125']).replace('sub_WALK', f['F-61830'])
                .replace('GIVE_ADDR', '0x%Xu' % GIVE).replace('GLOBAL_ADDR', '0x%Xu' % GLOBAL).replace('MASK_ADDR', '0x%Xu' % MASK))
    # the give (vtable slot 2): recorded in the manager's spare words (+64 count, +68.. the rewards); an experience reward
    # (+12 = its system, set below from the reward's id) walks that system's thresholds as AddExperience does
    give = ('uint32_t m=load(0x%Xu),c=load(m+64);store(m+68+4*c,ctx.r3.u32);store(m+64,c+1);'
            'uint32_t s=load(ctx.r3.u32+12);if(s){Context c2=ctx;c2.r3.u32=s;c2.r4.u32=0;%s(c2,base);}') % (GLOBAL, f['F-61830'])
    fakes = {GIVE: give}
    stubs = {share: 'uint32_t b=load(ctx.r3.u32+200);float x;memcpy(&x,&b,4);ctx.f1.f64=x;'}
    # the experience rewards' system pointer: set after the systems are built
    main_src = main_src.replace('  // the experience rewards: their system (+12)\n  for(uint32_t k=0;k<n;k++){}',
                                '  for(uint32_t k=0;k<n&&k<ns;k++)store(rw[k]+12,sy[k]);')
    exe = rd.build(idx, list(f.values()), main_src, work, 'rthr', image=img, fakes=fakes, stubs=stubs)
    sysline = lambda systems: '%x %s' % (len(systems), ' '.join('%g %x %s' % (sh, len(ths), ' '.join('%g %x %s' % (p, len(rs), ' '.join('%x' % k for k in rs)) for p, rs in ths)) for sh, ths in systems))
    line = lambda c: 'T %x %s ; %s ; %s' % (len(c[0]), ' '.join('%x %x %x' % x for x in c[0]), sysline(c[1]), ' '.join(c[2]))
    inp = '\n'.join('T %x %s ; %s %s ;' % (len(c[0]), ' '.join('%x %x %x' % x for x in c[0]), sysline(c[1]), ' '.join(c[2])) for c in cases) + '\n'
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[3:].strip() for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%s -> %s\n' % (line(c), o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
