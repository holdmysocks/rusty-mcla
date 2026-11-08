"""Run the Pursuit AlongSide's side pick F-14848 offline (static recompilation, native, flat guest memory) over every
answer of its five tests, for the port's `police.rs::side_pick` (test side_pick_matches_the_original).

F-14848 (asm, docs/reference/hookmen/triage_small_h4.md): the cop = tactic +76[0], its brain +876; the target's matrix
F-14038; p1 / p2 = position +- right x fAlongSideDist (tune +304); a / b = F-72689 (the nav test with the brain's filter
+164 - the police brain's mcCopFilter, which allows every material: the port's on_mesh) of p1 / p2; c / d = F-90092 (the
road filter: on_road) of p1 / p2; e = F-30098 (the target on the cop's right). Here the matrix is the identity at the
origin with fAlongSideDist 5, so p1.x = 5 and p2.x = -5 tell the two points apart in the stubs.

Output, one line a case: <a> <b> <c> <d> <e> <result>   (32 lines)
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t S=0x40800000u, F=0x40800100u, T=0x40800200u, L=0x40800300u, C=0x40800400u, B=0x40801000u, MX=0x40802000u;
 for(int m=0;m<32;m++){ rt_reset();
  sf(0x8282CAC8u+304,5.0f);
  store(S+12,F); store(F+20,T); store(T+76,L); store(L,C); store(C+876,B);
  for(int i=0;i<16;i++) sf(MX+4*i,(i%5==0)?1.0f:0.0f); sf(MX+60,1.0f);
  store(0x40C00000u,(uint32_t)m);
  Context c=fresh(); c.r3.u32=S; PICK(c,nullptr);
  printf("%d %d %d %d %d %d\n",m&1,(m>>1)&1,(m>>2)&1,(m>>3)&1,(m>>4)&1,(int)(c.r3.u32&0xff));}
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
    ids = ['F-14848', 'F-14038', 'F-72689', 'F-90092', 'F-30098']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    px = '([](uint32_t u){float f;memcpy(&f,&u,4);return f;})(load(%s))'
    bit = lambda k: '((load(0x40C00000u)>>%d)&1)' % k
    stubs = {
        name('F-14038'): 'ctx.r3.u64=0x40802000u;',
        name('F-72689'): 'ctx.r3.u64= %s>0.0f ? %s : %s;' % (px % 'ctx.r4.u32', bit(0), bit(1)),
        name('F-90092'): 'ctx.r3.u64= %s>0.0f ? %s : %s;' % (px % 'ctx.r3.u32', bit(2), bit(3)),
        name('F-30098'): 'ctx.r3.u64=%s;' % bit(4),
    }
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-14848')], MAIN.replace('PICK', name('F-14848')), a.out, 'side_pick', image=img,
                      stubs=stubs, limit=50)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout[:-len('end\n')])
    print(r.stdout, end='')


if __name__ == '__main__':
    main()
