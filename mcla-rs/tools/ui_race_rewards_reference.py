"""The original's race reward listener F-48818 (GENERIC / FREEWAY_RACE_COMPLETE) run offline (recomp_dispatch): what it hands to the
pending reward list. Writes ui_race_rewards_ref.txt, one case per line, numbers in hex:
  Q calls ; (exp money racers place arcade network type editor) x calls -> (kind amount hidden give_now) per list entry
      exp / money: the base experience (F-31900) and base money (F-78976) as float bits - stubbed, ported elsewhere; racers = the
      results' +3080, place = the player's entry +172, arcade = +3132, type = +3096 (10 = pink slip), editor = +3140; network =
      what F-24338 answers. The listener runs `calls` times on the same manager before anything is given (a race queued again).
      kind M = the money reward (+428, amount +60), P = Progression experience (+236, amount +92), U = Unlocking (+332).
      The experience manager's three rewards are built by their real constructors (F-78705, F-69550); AddReward F-64223 real.
      Stubbed: the player's entry lookup F-76117 (entry 0), F-69503, the class / ability lookups F-45718 / F-53701 / F-86078
      (no system), AddExperience F-55689, the rubber band F-20820, the money text formatter F-13827.
Usage: ui_race_rewards_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
GLOBAL = 0x82874848
# the case's values, in free arena words below the heap (read by the stubs)
CASE = 0x40000C00


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 char kind[4];
 while(scanf("%3s",kind)==1){
  rt_reset();
  uint32_t calls;scanf("%x",&calls);
  uint32_t rm=rt_alloc(256,16),em=rt_alloc(1024,16),res=rt_alloc(4096,16),veh=rt_alloc(256,16);
  for(uint32_t i=0;i<256;i+=4){store(rm+i,0);store(veh+i,0);}for(uint32_t i=0;i<1024;i+=4)store(em+i,0);
  store(GLOBAL_ADDR,rm);
  {Context c=fresh();c.r3.u32=em+236;sub_EXPCTOR(c,nullptr);}
  {Context c=fresh();c.r3.u32=em+332;sub_EXPCTOR(c,nullptr);}
  {Context c=fresh();c.r3.u32=em+428;sub_MONCTOR(c,nullptr);}
  char sep[4];scanf("%3s",sep);
  for(uint32_t k=0;k<calls;k++){
   uint32_t e,m,racers,place,arc,net,type,ed;scanf("%x %x %x %x %x %x %x %x",&e,&m,&racers,&place,&arc,&net,&type,&ed);
   for(uint32_t i=0;i<4096;i+=4)store(res+i,0);
   store(res+8,veh);store(res+172,place);store(res+3080,racers);store(res+3132,arc);store(res+3096,type);store(res+3140,ed);store(res+3100,1);
   store(CASE_ADDR+0,e);store(CASE_ADDR+4,m);store(CASE_ADDR+8,net);
   Context c=fresh();c.r3.u32=em;c.r4.u32=res;sub_RACE(c,nullptr);
  }
  printf("OUT");
  uint32_t cnt=load(rm+4)>>16,arr=load(rm);
  for(uint32_t i=0;i<cnt;i++){uint32_t r=load(arr+4*i);
   if(r==em+428)printf(" M %x",load(r+60));else if(r==em+236)printf(" P %x",load(r+92));else if(r==em+332)printf(" U %x",load(r+92));else printf(" ? 0");
   printf(" %x %x",(uint32_t)*pointer(r+8),(uint32_t)*pointer(r+9));}
  printf("\n");fflush(stdout);
 }
}
'''


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 600
    r = random.Random(48818)
    cases = []
    for _ in range(count):
        calls = 1 if r.randrange(4) else 2
        cs = []
        for _ in range(calls):
            racers = r.randrange(1, 9)
            exp = r.choice([0.0, 5.0, 9.99, r.uniform(0, 40), r.uniform(0, 3000), float(r.randrange(0, 3000))])
            money = r.choice([0.0, 1.0, r.uniform(0, 40), r.uniform(0, 20000), float(r.randrange(0, 20000))])
            cs.append((fbits(exp), fbits(money), racers, r.randrange(1, racers + 1), int(r.randrange(5) == 0), int(r.randrange(5) == 0),
                       10 if r.randrange(8) == 0 else r.randrange(0, 10), int(r.randrange(20) == 0)))
        cases.append(cs)
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_rrace'
    work.mkdir(exist_ok=True)
    f = {k: 'sub_' + addr(k) for k in ('F-48818', 'F-78705', 'F-69550')}
    st = lambda k: 'sub_' + addr(k)
    fl = lambda o: '{uint32_t b=load(0x%Xu);float x;memcpy(&x,&b,4);ctx.f1.f64=x;}' % (CASE + o)
    stubs = {st('F-76117'): 'store(ctx.r5.u32,0);', st('F-31900'): fl(0), st('F-78976'): fl(4),
             st('F-69503'): 'ctx.r3.u32=0;', st('F-24338'): 'ctx.r3.u32=load(0x%Xu);' % (CASE + 8), st('F-45718'): 'ctx.r3.u32=0x40000800u;',
             st('F-53701'): 'ctx.r3.u32=0;', st('F-86078'): 'ctx.r3.u32=0;', st('F-55689'): '', st('F-20820'): '',
             # (the money's description text: a formatter on a runtime object - text only)
             st('F-13827'): ''}
    main_src = (MAIN.replace('CASE_ADDR', '0x%Xu' % CASE).replace('sub_RACE', f['F-48818'])
                .replace('sub_EXPCTOR', f['F-78705']).replace('sub_MONCTOR', f['F-69550']).replace('GLOBAL_ADDR', '0x%Xu' % GLOBAL))
    # (the amount setters call vtable slot 1 of the money / experience reward: their description, run as is)
    slot1 = [rd.image_u32(img, vt + 4) for vt in (0x820BB8E0, 0x820BB850)]
    exe = rd.build(idx, list(f.values()), main_src, work, 'rrace', image=img, stubs=stubs, targets=slot1)
    line = lambda cs: 'Q %x ; %s' % (len(cs), ' '.join('%x %x %x %x %x %x %x %x' % c for c in cs))
    inp = '\n'.join(line(cs) for cs in cases) + '\n'
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[3:].strip() for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%s -> %s\n' % (line(c), o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
