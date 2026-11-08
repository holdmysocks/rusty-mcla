"""Run the patrol's path-change pick F-84839 offline (static recompilation, native, flat guest memory) for the port's
`police.rs::path_change` (test path_change_matches_the_original).

F-84839(mgr, unit, &path, &point): with a brain (+68) of the patrol kind (its vt+0 class test, here a vtable slot answering
1), a group (+1120 != -1) of more than one path (group = mgr +172 + 20 x +1120: paths +8, u16 count +12) and the unit's timer
+1128 <= 0: one step of the shared MWC (82860890: x = lo x 0x5CDCFAA7 + carry), its low 23 bits x 2^-23 against
iChangePathProbPrcnt (tune +320, int) x 0.01; above -> +1128 = 1.0, 0; else the nearest point (dy dy, dz, dx fused; strictly
under, from 9999979520) over the group's paths other than the unit's own (+1124; points +4 stride 16, u16 count +8), written
to the two outs (-1 when none), 1. The position comes from F-35658 (stubbed: the unit + 256, its +48).

Cases: 4,000 from a fixed seed - groups of 1 - 4 paths of 0 - 6 points, the own index -1 / in range, timers <= 0 / > 0, the
percent 0 / 1 / 50 / 100 / random, random MWC states. Output, one line a case (decimal):
  <n paths> <points per path, comma list> <own> <timer bits> <pct> <lo> <carry> <unit x y z bits> <points' x y z bits, all,
  comma list> | <result> <path> <point> <timer bits after> <lo after> <carry after>
"""
import argparse
import random
import re
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t M=0x40800000u, G=0x40801000u, PA=0x40802000u, PH=0x40803000u, PT=0x40810000u,
  U=0x40900000u, BR=0x40910000u, BV=0x40911000u, O1=0x40920000u, O2=0x40920010u;
 while(in>>tok){ if(tok!="case") break; uint32_t np, own, tb, pct, lo, ca, ux, uy, uz; in>>np; uint32_t cnt[8]; for(uint32_t j=0;j<np;j++) in>>cnt[j];
  in>>own>>tb>>pct>>lo>>ca>>ux>>uy>>uz; rt_reset();
  store(M+172,G); store(G+8,PA); store16(G+12,(uint16_t)np);
  uint32_t q=PT;
  for(uint32_t j=0;j<np;j++){ uint32_t P=PH+32*j; store(PA+4*j,P); store(P+4,q); store16(P+8,(uint16_t)cnt[j]);
   for(uint32_t k=0;k<cnt[j];k++){ uint32_t x,y,z; in>>x>>y>>z; store(q,x); store(q+4,y); store(q+8,z); store(q+12,0); q+=16; } }
  store(U+68,BR); store(BR,BV); store(BV,0x82130008u); store(U+1120,0); store(U+1124,own); store(U+1128,tb);
  store(U+256+48,ux); store(U+256+52,uy); store(U+256+56,uz); store(U+256+60,0);
  store(0x8282CAC8u+320,pct); store(0x82860890u,lo); store(0x82860894u,ca); store(O1,0x77777777u); store(O2,0x77777777u);
  Context c=fresh(); c.r3.u32=M; c.r4.u32=U; c.r5.u32=O1; c.r6.u32=O2; PICK(c,nullptr);
  printf("%u %d %d %u %u %u\n",(unsigned)(c.r3.u32&0xff),(int)load(O1),(int)load(O2),load(U+1128),load(0x82860890u),load(0x82860894u));}
 printf("end\n");
}
'''


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def cases():
    g = random.Random(84839)
    out = []
    for k in range(4000):
        np = g.choice([1, 1, 2, 2, 3, 4])
        cnt = [g.randint(0, 6) for _ in range(np)]
        own = g.choice([-1 & 0xFFFFFFFF, g.randint(0, np - 1), g.randint(0, np - 1)])
        tb = bits(g.choice([0.0, -0.0, -0.5, g.uniform(-2, 0), 0.25, g.uniform(0, 3)]))
        pct = g.choice([0, 1, 50, 100, g.randint(0, 100)])
        lo, ca = g.getrandbits(32), g.getrandbits(32) % 0x5CDCFAA7
        u = [bits(g.uniform(-1500, 1500)), bits(g.uniform(-10, 60)), bits(g.uniform(-1500, 1500))]
        pts = []
        for j in range(np):
            for _ in range(cnt[j]):
                if g.random() < 0.1 and pts:
                    pts.append(list(pts[g.randrange(len(pts))]))  # a tie
                else:
                    pts.append([bits(g.uniform(-1500, 1500)), bits(g.uniform(-10, 60)), bits(g.uniform(-1500, 1500))])
        out.append((np, cnt, own, tb, pct, lo, ca, u, pts))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-84839', 'F-35658', 'F-49619']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-35658'): 'ctx.r3.u64=ctx.r3.u32+256u;'}
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-84839')], MAIN.replace('PICK', name('F-84839')), a.out, 'path_change', image=img,
                      stubs=stubs, targets=[addr['F-49619']], limit=30)
    cs = cases()
    cf = a.out / 'path_cases.txt'
    with open(cf, 'w') as f:
        for (np, cnt, own, tb, pct, lo, ca, u, pts) in cs:
            f.write('case %d %s %d %d %d %d %d %d %d %d %s\n' % (np, ' '.join(map(str, cnt)), own, tb, pct, lo, ca, *u,
                                                             ' '.join('%d %d %d' % tuple(p) for p in pts)))
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    res = r.stdout.splitlines()[:-1]
    # (8-digit numbers padded to 10 digits, so that no word reads as an address of the original to the citation check)
    pad = lambda t: re.sub(r'\b(\d{8})\b', lambda m: m.group(1).zfill(10), t)
    with open(a.ref, 'w') as f:
        for (np, cnt, own, tb, pct, lo, ca, u, pts), line in zip(cs, res):
            f.write(pad('%d %s %d %d %d %d %d %d %d %d %s | %s\n' % (np, ','.join(map(str, cnt)), own, tb, pct, lo, ca, *u,
                                                                ','.join('%d,%d,%d' % tuple(p) for p in pts) or '-', line)))
    print('cases', len(res), 'picked', sum(l.startswith('1 ') for l in res), '->', a.ref)


if __name__ == '__main__':
    main()
