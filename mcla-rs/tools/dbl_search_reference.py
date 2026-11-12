"""Run the original's Double Pursuit Search enter (F-68396) offline and record it, for the port's `police::dbl_search_nearer` /
`dbl_search_turn` / `search_block` (crates/game/src/testdata/dbl_search_ref.txt; test police::tests::dbl_search_matches_the_original).

F-68396 (r3 the state: +12 -> +20 the tactic: +32 the target car, +36 its word, +76 the two cops' list; +16 the timer it
clears) runs unchanged with the command block's init F-60530; the brain command F-24671 is stubbed and logs the cop, the
command and the block's words +12 .. +28 (+0 / +4 / +8 are the target, the tactic's +36 and the tactic: the port's blocks
carry none). Each car as tools/squad_swipe_reference.py lays it out (the matrix through +48 -> +28 -> +16). The shared
generator (the multiply-with-carry at its image place, lo and carry) is set per case and read after.

  t<x,y,z> a<cop 0 x,y,z> b<cop 1 x,y,z> g<lo> g<carry> -> <cop cmd w12 w16 w20 w24 w28> <cop ...> +16 <bits> g<lo> g<carry>
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MWC = 0x82860890                # the shared generator (lo, carry)

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t S=0x40800000u,F=0x40810000u,TA=0x40820000u,L=0x40830000u,TV=0x40A00000u,C0=0x40B00000u,C1=0x40C00000u;
 auto car=[](uint32_t v){store(v+48,v+0x400);store(v+0x400+28,v+0x800);store(v+0x400+8,v+0xC00);};
 auto rd=[&](uint32_t a){uint32_t u; in>>std::hex>>u>>std::dec; store(a,u);};
 while(in>>tok){
  if(tok!="case") break; rt_reset();
  for(uint32_t a=S;a<S+0x40000;a+=4) store(a,0);
  car(TV); car(C0); car(C1);
  for(int i=0;i<3;i++) rd(TV+0x800+16+48+4*i); for(int i=0;i<3;i++) rd(C0+0x800+16+48+4*i); for(int i=0;i<3;i++) rd(C1+0x800+16+48+4*i);
  rd(MWCA); rd(MWCA+4);
  store(S+12,F); store(F+20,TA); store(S+16,0xDEADBEEFu); store(TA+32,TV); store(TA+36,0x1234u); store(TA+76,L); store(L,C0); store(L+4,C1);
  Context c=fresh(); c.r3.u32=S; ENTER(c,nullptr);
  printf("+16 %08x g%08x g%08x\n",load(S+16),load(MWCA),load(MWCA+4)); }
 printf("end\n");
}
'''


def f32(v):
    return struct.unpack('<f', struct.pack('<f', v))[0]


def fbits(v):
    return struct.unpack('<I', struct.pack('<f', v))[0]


def cases():
    g = random.Random(68396)
    out = []
    for _ in range(3000):
        t = [f32(g.uniform(-3000, 3000)), f32(g.uniform(-10, 100)), f32(g.uniform(-3000, 3000))]
        cop = lambda r: [f32(t[0] + g.uniform(-r, r)), f32(t[1] + g.uniform(-3, 3)), f32(t[2] + g.uniform(-r, r))]
        a = cop(g.choice((5, 50, 400)))
        u = g.random()
        if u < 0.1:                                     # (a tie: the same place)
            b = list(a)
        elif u < 0.2:                                   # (a tie by symmetry: mirrored through the target in x / z)
            b = [f32(2 * t[0] - a[0]), a[1], f32(2 * t[2] - a[2])]
        elif u < 0.3:                                   # (the last bit apart)
            b = [struct.unpack('<f', struct.pack('<I', fbits(a[0]) + g.choice((1, -1))))[0], a[1], a[2]]
        else:
            b = cop(g.choice((5, 50, 400)))
        out.append((t, a, b, g.getrandbits(32), g.getrandbits(32) % 0x5CDCFAA7))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-68396', 'F-24671', 'F-60530']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-24671'): 'printf("%u %u",ctx.r3.u32==0x40B00000u?0u:ctx.r3.u32==0x40C00000u?1u:9u,ctx.r4.u32);'
                              ' for(uint32_t o=12;o<32;o+=4) printf(" %08x",load(ctx.r5.u32+o)); printf(" ");'}
    src = MAIN.replace('ENTER', name('F-68396')).replace('MWCA', '0x%08Xu' % MWC)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-68396')], src, a.out, 'dblsearch', image=img, stubs=stubs)
    cs = cases()
    cf = a.out / 'dblsearch_cases.txt'
    xs = lambda v: ' '.join('%08x' % fbits(q) for q in v)
    with cf.open('w') as f:
        for t, p0, p1, lo, ca in cs:
            f.write('case %s %s %s %08x %08x\n' % (xs(t), xs(p0), xs(p1), lo, ca))
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.split('\n')[:-2]
    assert len(lines) == len(cs)
    xc = lambda v: ','.join('%08x' % fbits(q) for q in v)
    rows = ['t%s a%s b%s g%08x g%08x -> %s' % (xc(t), xc(p0), xc(p1), lo, ca, l) for (t, p0, p1, lo, ca), l in zip(cs, lines)]
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
