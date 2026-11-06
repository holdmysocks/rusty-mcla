"""Run the original's brain command F-83687 offline (static recompilation, native, flat guest memory) and record what it writes
into the cop's brain, for the port's test (crates/game/src/testdata/brain_command_ref.txt; test brain_command_matches_the_original).

F-83687 is what the cop command dispatcher F-24671 calls for every command but 2 (Follow): it copies the command block into the
brain and resets its command timers, then changes the brain machine's state. It runs unchanged on a brain whose words +560..+780
and the racing brain's +292 are random before each call (so a field it does not write shows as kept), the machine at +560 a stub
object in state 0, -1 or 3 (its reset, slot 3, recorded: F-61449 / the start are compared by tools/brain_build_reference.py), the
route clear F-69438 (command 0) and the state change F-17363 stubbed and recorded (their own rows); the racing brain's final
speed setter F-39608 (+292) runs. Blocks: 32 random words, commands 0..9 (9 is out of range: only the common part), the
previous command +568 random 0..8 (0 in a quarter of the calls), some calls with no block (r5 = 0: only reached for command 0).

Inputs come from a 32-bit xorshift (seed 20261106) the port's test runs too: per call the command (0..9), for command 0 a third of
the calls with no block, the 8 block words, the machine's state (0 / -1 / 3), the brain words, the previous command (0 in a quarter), +292. First the three blocks the port's senders build, from 32 bytes of A5 (BLOCK basic / stop / along <8 words>). Output per call:
  <k> <cmd> <offset>=<word> ... (every brain word that changed) [292=<word>] route=<0|1> reset=<the machine's reset argument, or -1> state=<asked or -1>
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

N = 4000
MAIN_IDS = ['F-83687']
STUBS = ['F-69438', 'F-17363']
BLOCKS = ['F-59773', 'F-72181', 'F-94949', 'F-60530']
LO, HI = 568, 780   # the brain words compared

MAIN = r'''
static uint32_t X=20261106u; static uint32_t nx(){X^=X<<13;X^=X>>17;X^=X<<5;return X;}
static uint32_t word(){uint32_t r=nx(),u=r%10; static const uint32_t F[8]={0u,0x3F800000u,0xBF800000u,0xC47A0000u,0x3F000000u,0x40200000u,0xBE800000u,0x3F000001u};
 static const uint32_t I[4]={0u,1u,0xFFFFFFFFu,0x01000000u}; if(u<3)return F[(r>>8)%8]; if(u<5)return I[(r>>8)%4]; return (r>>24)==0x82u?r^0x01000000u:r;}   // (no word that reads as an address of the original)
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t B=0x40A00000u, MC=0x40A10000u, BL=0x40A20000u, MVT=0x40A30000u; const int n=atoi(argv[2]);
 for(int k=0;k<n;k++){ rt_reset(); uint32_t cmd=nx()%10; bool none=cmd==0&&nx()%3==0;
  uint32_t cur=nx()%3; cur=cur==0?0u:cur==1?0xFFFFFFFFu:3u; store(MC,MVT); store(MVT+12,0x7F000020u); store(MC+12,cur); store(B+560,MC); store(B+564,0);
  if(!none) for(int i=0;i<8;i++) store(BL+4*i,word());
  uint32_t pre[(HI-LO)/4]; for(int o=LO;o<HI;o+=4){ pre[(o-LO)/4]=word(); store(B+o,pre[(o-LO)/4]); }
  uint32_t prev=nx()%4==0?0u:nx()%9; pre[0]=prev; store(B+LO,prev); uint32_t f292=word(); store(B+292,f292);
  store(0x40C00000u,0); store(0x40C00004u,0xFFFFFFFFu); store(0x40C00008u,0xFFFFFFFFu);
  Context c=fresh(); c.r3.u32=B; c.r4.u32=cmd; c.r5.u32=none?0:BL; CMD(c,nullptr);
  printf("%d %u",k,cmd); for(int o=LO;o<HI;o+=4) if(load(B+o)!=pre[(o-LO)/4]) printf(" %d=%08x",o,load(B+o));
  if(load(B+292)!=f292) printf(" 292=%08x",load(B+292)); printf(" route=%u reset=%d state=%d\n",load(0x40C00000u),(int)load(0x40C00008u),(int)load(0x40C00004u)); }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = MAIN_IDS + STUBS + BLOCKS
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-69438'): 'store(0x40C00000u,1);', name('F-17363'): 'store(0x40C00004u,ctx.r4.u32);'}
    main_src = MAIN.replace('CMD', name('F-83687')).replace('LO', str(LO)).replace('HI', str(HI))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in MAIN_IDS], main_src, a.out, 'brain_command', image=img, limit=400, stubs=stubs,
                      fakes={0x7F000020: 'store(0x40C00008u,ctx.r4.u32);'})
    r = subprocess.run([str(exe), str(a.image), str(N)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    # the command blocks the port's senders build (cmd_block): F-59773 (Pursue / Pull Over), F-72181 (Stop), F-94949
    # (AlongSide, +20 the tune's fAlongSideDist - in the image the class default 3.5), F-60530 (Search, before the caller's side
    # and angle), each on 32 bytes of A5
    bm = (r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t BL=0x40A20000u; void(*f[4])(Context&,const uint8_t*)={B0,B1,B2,B3}; const char*n[4]={"basic","stop","along","search"};
 for(int k=0;k<4;k++){ rt_reset(); for(int i=0;i<8;i++) store(BL+4*i,0xA5A5A5A5u); Context c=fresh(); c.r3.u32=BL; f[k](c,nullptr);
  printf("BLOCK %s",n[k]); for(int i=0;i<8;i++) printf(" %08x",load(BL+4*i)); printf("\n"); }
}
'''.replace('B0', name('F-59773')).replace('B1', name('F-72181')).replace('B2', name('F-94949')).replace('B3', name('F-60530')))
    bexe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in BLOCKS], bm, a.out, 'cmd_blocks', image=img, limit=50)
    rb = subprocess.run([str(bexe), str(a.image)], capture_output=True, text=True, check=True)
    a.ref.write_text(rb.stdout + r.stdout)
    print('calls', N, '->', a.ref)


if __name__ == '__main__':
    main()
