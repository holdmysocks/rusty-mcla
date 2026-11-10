"""Run the original's cop zone pick offline and record it, for the port's `police::zone_pick`
(crates/game/src/testdata/zone_pick_ref.txt; test police::tests::zone_pick_matches_the_original).

EvaluateCopZones' body F-88125 (the native's handler jumps to it): with no police manager (image +0x86D8E0) nothing. Else the
zone state +196 = 0, the zone +188 = -1, the lap +192 = -1, +212 = 0.0, the target +200 = 0. The dev override F-47443 (stubbed:
the case's answer and value) set with a value <= 0, no police in the manager's list (F-53190, stubbed: the case's count) or
no zones (+184, u16) - "no cops": nothing more. Otherwise one index draw of the shared generator (the MWC at image +0x860890,
a = 0x5CDCFAA7: (lo & 0x7FFFFFFF) % n) picks the zone, the lap is 0 and, with a race current (F-27863, stubbed: the case's
race; its +3124 the laps - the code's test of race + 8 is on the address, so never fails) of more than one lap, a second draw % laps. Lap 0 arms it (+196 = 1).
Then SetAllPoliceUnderground F-36452 (1, 1) (stubbed: recorded) and the log.

  <ovr ans> <ovr val> <police> <zones> <race 0 none / 1 with the word +8 = 1 / 2 with +8 = 0> <laps> g<lo> g<carry> ->
  <+188> <+192> <+196> <+200> p<+212 bits> <underground 0 / 1> g<lo> g<carry>
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
MGRG = IMG + 0x86D8E0
RNG = IMG + 0x860890

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t M=0x40800000u, R=0x40900000u, G0=0x40A00000u;
 while(in>>tok){ if(tok!="case") break; uint32_t oa,ov,np,nz,rc,laps,lo,ca; in>>oa>>ov>>np>>nz>>rc>>laps>>lo>>ca;
  rt_reset(); for(uint32_t a=M;a<M+256;a+=4) store(a,0x11111111u); for(uint32_t a=G0;a<G0+64;a+=4) store(a,0);
  store(MGRG,M); store(M+180,0x40810000u); for(int i=0;i<64;i++) store(0x40810000u+4*i,0x40820000u);
  store(M+184,(nz<<16)|(load(M+184)&0xffffu));
  store(G0,oa); store(G0+4,ov); store(G0+8,np); store(G0+12, rc==0?0u:R);
  store(R+8, rc==1?1u:0u); store(R+3124,laps);
  store(RNG,lo); store(RNG+4,ca);
  Context c=fresh(); PICK(c,nullptr);
  printf("%u %u %u %u %u %u %u %u -> %d %d %u %u %08x %u %u %u\n",oa,ov,np,nz,rc,laps,lo,ca,(int)load(M+188),(int)load(M+192),
         load(M+196),load(M+200),load(M+212),load(G0+16),load(RNG),load(RNG+4)); }
 printf("end\n");
}
'''


def cases():
    g = random.Random(88125)
    seed = lambda: (g.getrandbits(32), g.getrandbits(32) % 0x5CDCFAA7)
    out = []
    for oa, ov in ((0, 0), (1, 0), (1, 1), (1, 0xFFFFFFFF), (1, 5)):
        for np in (0, 1, 3):
            for nz in (0, 1, 2, 3, 5, 7):
                for rc in (0, 1, 2):
                    for laps in (0, 1, 2, 3, 5, 0xFFFFFFFF):
                        out.append((oa, ov, np, nz, rc, laps) + seed())
    for _ in range(1500):
        out.append((0, 0, g.randrange(1, 6), g.randrange(1, 12), 1, g.randrange(0, 9)) + seed())
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-88125', 'F-47443', 'F-53190', 'F-27863', 'F-36452', 'F-47121']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-47443'): 'store(ctx.r3.u32,load(0x40A00004u)); ctx.r3.u64=load(0x40A00000u);',
             name('F-53190'): 'ctx.r3.u64=load(0x40A00008u);',
             name('F-27863'): 'ctx.r3.u64=load(0x40A0000Cu);',
             name('F-36452'): 'store(0x40A00010u, 1u + ((ctx.r3.u32==1 && ctx.r4.u32==1)?0u:8u));',
             name('F-47121'): ''}
    src = MAIN.replace('PICK', name('F-88125')).replace('MGRG', '0x%08Xu' % MGRG).replace('RNG', '0x%08Xu' % RNG)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-88125')], src, a.out, 'zone_pick', image=img, stubs=stubs)
    cs = cases()
    cf = a.out / 'zone_cases.txt'
    cf.write_text(''.join('case ' + ' '.join(str(v) for v in c) + '\n' for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    rows = []
    for l in r.stdout.split('\n')[:-2]:
        lhs, rhs = l.split(' -> ')
        v, w = lhs.split(), rhs.split()
        # (every hex word with a letter prefix: no number in the file reads as a code address)
        rows.append('%s g%08x g%08x -> %s p%s %s g%08x g%08x' % (' '.join(v[:6]), int(v[6]), int(v[7]), ' '.join(w[:4]), w[4], w[5],
                                                                 int(w[6]), int(w[7])))
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
