"""Run the original's police level PoliceManager_CheckPoliceActivity (F-35540) offline and record it, for the port's
`police::activity_rule` (crates/game/src/testdata/activity_ref.txt; test police::tests::activity_matches_the_original).

F-35540 (r3 the native's argument != 0, as its handler passes it) with the police manager at image +0x86D8E0: no manager 0;
a Dispatch (the tactic of type 0 F-10826 finds for the argument) whose +160's +12 is above 0 (signed) -1; then a tactic of the
argument in its Search with the clock running (F-63053) 3, in its Search (F-77780) 2; the manager's +56 byte 100; with the
argument set and the manager's +196 (the cop zone's state) 2 or 3, or a tactic of type 4, 7; types 3 / 2 / 1 6 / 5 / 4; then the
units (F-53190's count, F-45180's i-th): one whose +872 equals the argument and whose command (F-86113 / F-57281, run as they
are: the brain +876's +568, or without a brain the cop's +1076 != 0 as 2) is 2 (Follow) 1; else 0. The tactic finder and the
two Search tests are stubbed from the case; the unit getters F-86113 / F-57281 run unchanged.

  <arg> d<- / the Dispatch's +160 +12> <search clock> <search> <+56> <+196> t<types 1..4 present, a bit each>
  <units: side:brain cmd or - :+1076 ...> -> <level>
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
MGRG = IMG + 0x86D8E0           # the police manager

MAIN = r'''
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t M=0x40800000u, G0=0x40A00000u, D=0x40810000u, U=0x40B00000u;
 while(in>>tok){
  if(tok!="case") break; uint32_t mgr,arg,dsp,dv,sc,ss,held,z,types,nu; in>>mgr>>arg>>dsp>>dv>>sc>>ss>>held>>z>>types>>nu;
  rt_reset(); for(uint32_t a=M;a<M+256;a+=4) store(a,0); for(uint32_t a=G0;a<G0+64;a+=4) store(a,0);
  store(MGRG,mgr?M:0u); sb(M+56,held); store(M+196,z);
  // (the tactic finder's answers by type 0..4 at G0+0.., the two Search tests at G0+20 / +24, the units' count at +28)
  store(G0,dsp?D:0u); for(int t=1;t<=4;t++) store(G0+4*t,(types>>(t-1)&1)?0x40820000u+t*0x100u:0u);
  store(D+160,D+0x100); store(D+0x100+12,dv); store(G0+20,sc); store(G0+24,ss); store(G0+28,nu);
  for(uint32_t i=0;i<nu;i++){ uint32_t side,hb,cmd,w; in>>side>>hb>>cmd>>w; uint32_t u=U+i*0x1000u;
   for(uint32_t a=u+864;a<u+1088;a+=4) store(a,0); sb(u+872,side); store(u+876,hb?u+0x800u:0u); store(u+0x800u+568,cmd); store(u+1076,w); }
  CALL }
 printf("end\n");
}
'''


def cases():
    g = random.Random(35540)
    out = []
    for _ in range(3000):
        mgr = 0 if g.random() < 0.03 else 1
        arg = g.choice((1, 1, 1, 0))
        dsp = g.random() < 0.15
        dv = g.choice((0, 1, 2, 3, -1, 0x7FFFFFFF, -0x80000000)) & 0xFFFFFFFF
        sc, ss = (g.random() < 0.1), (g.random() < 0.15)
        held = g.choice((0, 0, 0, 0, 1, 0x80))
        z = g.choice((0, 0, 1, 2, 3, 4, -1)) & 0xFFFFFFFF
        types = g.getrandbits(4) if g.random() < 0.5 else 0
        units = []
        for _ in range(g.choice((0, 1, 2, 3, 3, 5))):
            side = g.choice((1, 1, 1, 0, 2))
            hb = g.random() < 0.7
            cmd = g.choice((0, 1, 2, 2, 3, 7, 8, 0xFFFFFFFF))
            w = g.choice((0, 0, 1, 0xFFFFFFFF))
            units.append((side, int(hb), cmd, w))
        out.append((mgr, arg, int(dsp), dv, int(sc), int(ss), held, z, types, units))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--handler', action='store_true',
                    help='call the native handler F-80270 (PoliceManager_CheckPoliceActivity) with a native context instead '
                         'of F-35540: an argument of 1 becomes a random non-zero word; --ref is then compared with, not written')
    a = ap.parse_args()
    ids = ['F-80270', 'F-35540', 'F-10826', 'F-63053', 'F-77780', 'F-53190', 'F-45180', 'F-86113', 'F-57281']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {
        name('F-10826'): 'ctx.r3.u64=(ctx.r4.u32<=4u)?load(0x40A00000u+4u*ctx.r4.u32):0u;',
        name('F-63053'): 'ctx.r3.u64=load(0x40A00014u);',
        name('F-77780'): 'ctx.r3.u64=load(0x40A00018u);',
        name('F-53190'): 'ctx.r3.u64=load(0x40A0001Cu);',
        name('F-45180'): 'ctx.r3.u64=0x40B00000u+ctx.r4.u32*0x1000u;',
    }
    if a.handler:
        # (the native context: +0 the return slot's address, +8 the arguments' address; the handler writes F-35540(arg != 0)
        # to the slot - a marker there first, so a missing write shows)
        call = ('const uint32_t H=0x40C00000u; store(H+0x40,0xDEADBEEFu); store(H+0x80,arg); store(H,H+0x40); store(H+8,H+0x80);'
                ' Context c=fresh(); c.r3.u32=H; %s(c,nullptr); printf("%%d\\n",(int)load(H+0x40));' % name('F-80270'))
        roots = [name('F-80270'), name('F-35540')]
    else:
        call = 'Context c=fresh(); c.r3.u32=arg; %s(c,nullptr); printf("%%d\\n",(int)c.r3.u32);' % name('F-35540')
        roots = [name('F-35540')]
    src = MAIN.replace('CALL', call).replace('MGRG', '0x%08Xu' % MGRG)
    exe = rdisp.build(rdisp.index(a.recomp), roots + [name('F-86113'), name('F-57281')], src, a.out,
                      'activity_h' if a.handler else 'activity', image=img, stubs=stubs)
    cs = cases()
    g = random.Random(80270)
    words = [(g.choice((1, 2, 0x80000000, 0xFFFFFFFF, g.getrandbits(32) | 1)) if c[1] else 0) for c in cs]
    cf = a.out / 'activity_cases.txt'
    with cf.open('w') as f:
        for (mgr, arg, dsp, dv, sc, ss, held, z, types, units), w in zip(cs, words):
            f.write('case %d %d %d %d %d %d %d %d %d %d %s\n' % (mgr, w if a.handler else arg, dsp, dv, sc, ss, held, z, types, len(units),
                                                             ' '.join('%d %d %d %d' % u for u in units)))
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.split('\n')[:-2]
    assert len(lines) == len(cs)
    s32 = lambda v: v - (1 << 32) if v >= 0x80000000 else v
    rows = []
    for (mgr, arg, dsp, dv, sc, ss, held, z, types, units), l in zip(cs, lines):
        us = ' '.join('%d:%s:%d' % (side, s32(cmd) if hb else '-', s32(w)) for side, hb, cmd, w in units) or '.'
        rows.append('%s%d d%s %d %d %d %d t%d %s -> %s' % ('' if mgr else 'x', arg, s32(dv) if dsp else '-', sc, ss, held, s32(z), types, us, l))
    if a.handler:
        ref = a.ref.read_text().splitlines()
        bad = [i for i, (x, y) in enumerate(zip(rows, ref)) if x != y]
        print('handler: cases', len(rows), 'reference rows', len(ref), 'differ', len(bad), 'non-zero words',
              sum(1 for w in words if w), 'of them not 1', sum(1 for w in words if w > 1))
        for i in bad[:10]:
            print('  ', i, rows[i], '| ref', ref[i])
        sys.exit(1 if bad or len(ref) != len(rows) else 0)
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
