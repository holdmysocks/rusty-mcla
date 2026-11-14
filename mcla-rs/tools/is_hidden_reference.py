"""Run the original's mcHookman_IsHidden handler (F-30864) offline and record it, for the port's `Man::is_hidden`
(crates/game/src/testdata/is_hidden_ref.txt; test roamers::tests::is_hidden_is_the_originals_rule).

F-30864 takes the native context in r3: +8 the arguments' address (the first word the hookman), +0 the return slot's address.
It answers 0 without a hookman, or while the career block's (the word at image +0x86D8DC) byte +0x16C89 (65536 | 27785; the unlock-all byte,
F-47441) is set; else the hookman's byte +196 (mcHookman_Hide F-55720 writes arg != 0 there). Every case runs on a fresh
memory; the return slot holds a marker first, so a missing write shows.

  <hookman: 0 none / 1 there> <+196 byte> <unlock-all byte> -> <the word written to the return slot>
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
CAREERG = IMG + 0x86D8DC        # the career block's pointer

MAIN = r'''
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t H=0x40C00000u, M=0x40800000u, G=0x40900000u;
 while(in>>tok){
  if(tok!="case") break; uint32_t man,b196,unl; in>>man>>b196>>unl;
  rt_reset(); for(uint32_t a=M;a<M+256;a+=4) store(a,0); store(G+93320u,0); store(G+93324u,0);
  store(CAREERG,G); sb(G+93321u,(uint8_t)unl); sb(M+196u,(uint8_t)b196);
  store(H+0x40,0xDEADBEEFu); store(H+0x80,man?M:0u); store(H,H+0x40); store(H+8,H+0x80);
  Context c=fresh(); c.r3.u32=H; HND(c,nullptr);
  printf("%u\n",load(H+0x40)); }
 printf("end\n");
}
'''


def cases():
    out = []
    for man in (0, 1):
        for b196 in (0, 1, 2, 0x80, 0xFF):
            for unl in (0, 1, 0x80, 0xFF):
                out.append((man, b196, unl))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    r = subprocess.run([a.tool, 'registry', 'addr', 'F-30864'], capture_output=True, text=True, check=True).stdout
    addr = int([l for l in r.splitlines() if l.startswith('F-')][0].split()[1], 16)
    name = 'sub_%08X' % addr
    img = rdisp.load_image(a.image)
    src = MAIN.replace('HND', name).replace('CAREERG', '0x%08Xu' % CAREERG)
    exe = rdisp.build(rdisp.index(a.recomp), [name], src, a.out, 'is_hidden', image=img)
    cs = cases()
    cf = a.out / 'is_hidden_cases.txt'
    with cf.open('w') as f:
        for c in cs:
            f.write('case %d %d %d\n' % c)
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.split('\n')[:-2]
    assert len(lines) == len(cs)
    a.ref.write_text(''.join('%d %d %d -> %s\n' % (c + (l,)) for c, l in zip(cs, lines)))
    print('cases', len(cs), '->', a.ref)


if __name__ == '__main__':
    main()
