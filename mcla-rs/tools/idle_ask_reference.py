"""Run the cop brain's Idle command handler F-44966 (mcPBStateIdle vt+84) offline (static recompilation, native, flat guest
memory) for the port's `police.rs::idle_ask` (test idle_ask_matches_the_original).

F-44966(state, command): command > 8 (unsigned) returns; else its jump table: 0 -> the machine's (+12) pending state +16 = -1;
2 -> nothing; the others ask the machine for a state through SetNextState F-28114 (stubbed here: it records the state asked
and that the machine was passed). Commands: 0 - 12, and 4 large / negative words.

Output, one line a command: <command> <state asked or -2 for none> <machine +16 after, as a signed word; 7 = untouched sentinel>
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

CMDS = list(range(13)) + [100, 2147483647, 4294967295, 4294967288]

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t S=0x40800000u, M=0x40801000u; const uint32_t cmds[]={CMDS};
 for(uint32_t cmd: cmds){ rt_reset(); store(S+12,M); store(M+16,7u); store(0x40C00000u,0xFFFFFFFEu); store(0x40C00004u,0);
  Context c=fresh(); c.r3.u32=S; c.r4.u32=cmd; IDLE(c,nullptr);
  if(load(0x40C00000u)!=0xFFFFFFFEu && load(0x40C00004u)!=M){ printf("bad machine\n"); return 1; }
  printf("%u %d %d\n",cmd,(int)load(0x40C00000u),(int)load(M+16));}
 printf("end\n");
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
    ids = ['F-44966', 'F-28114']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-28114'): 'store(0x40C00000u,ctx.r4.u32); store(0x40C00004u,ctx.r3.u32);'}
    src = MAIN.replace('IDLE', name('F-44966')).replace('CMDS', ','.join('%du' % c for c in CMDS))
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-44966')], src, a.out, 'idle_ask', image=img, stubs=stubs, limit=20)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout[:-len('end\n')])
    print(r.stdout, end='')


if __name__ == '__main__':
    main()
