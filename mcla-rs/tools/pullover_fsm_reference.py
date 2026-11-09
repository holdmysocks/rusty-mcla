"""Run the original's pull-over tactic offline (static recompilation, native, flat guest memory) and record its machine for
the port's test (crates/game/src/testdata/pullover_fsm_ref.txt; test pullover_machine_matches_the_original).

The tactic object is built as the game builds it: ctor F-64611 (its FSM and the states CatchUp, AvoidCollision, Halt,
Ticket), reset F-92837, its cop and the target set as the engage sets them (+76, +32, active +20), begin F-74547, then the
tactic update F-36876 with dt = 1/60 s, N times; the FSM's code and the states' enters and ticks run unchanged. The tune gets
fPullOverDist 15 (+196, read through the tactic's slot 8 F-66453). The world is per-update inputs: every cop within F-90575
(14 m), the target moving F-96080 / F-43975 (one rule), closing F-32902 by its radius (0, 7, 50 m), the proximity test
F-29545 by its radius (50, 15 m), the side pick F-14848, the re-pick gate (tactic +36 -> +56), the cop's speed F-81535, and
the cop at the origin and the target at (x, 0, z) for Halt's squared distance (the matrix getter F-35658). The cop is valid
(F-14415) and the arrest test F-36806 false (Ticket is the port's heat). The brain command F-24671 is stubbed to what its
set-state does for these fields (+568 the command; Stop 1 the handbrake byte +688, AlongSide 5 the side +708); the cop's
velocity scale (F-82234 -> F-19124) is recorded as the product of the factors applied in the update.

Inputs come from an integer xorshift (the test makes the same ones): the flags as random walks, the cop's speed in quarters of
a m/s, the target's x / z in quarters of a metre as random walks.

  CASE <seed>
  <k> <state> <asked> <time bits> <+16 bits> <+20 bits> <brain +568> <+708> <+688> <velocity factor bits>   the updates where
  the state, the asked state, the brain or the factor changed, every 25th and the last
  (state: 0 CatchUp, 1 AvoidCollision, 2 Halt; +16: AvoidCollision's side timer, Halt's last squared distance; +20: Halt's
  growing time; 0 in other states)

Run it with the tactic's own vtable targets compiled in and the rest of their callees stubbed (the shared builder no
longer compiles these by itself; without them the build stops on an unresolved indirect call), from mcla-rs:

  python tools/pullover_fsm_reference.py --recomp <recomp> --image <image> --out <dir> --ref <file>
    --targets F-49619 F-74264 F-29906 F-24596 F-94002 F-74492 F-87882 F-54202 F-37056 F-82134 F-33778 F-77220 F-66453
    --more F-35685 F-75034 F-29294 F-38446 F-62425 F-29290 F-33229 F-62311

With these the tool writes crates/game/src/testdata/pullover_fsm_ref.txt byte for byte.
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

CASES = 40
N = 1200
MAIN_IDS = ['F-64611', 'F-92837', 'F-74547', 'F-36876']
WORLD = ['F-90575', 'F-96080', 'F-43975', 'F-32902', 'F-29545', 'F-14848', 'F-81535', 'F-35658', 'F-14415', 'F-36806', 'F-63286',
         'F-24671', 'F-82234', 'F-19124', 'F-58846', 'F-72181', 'F-59773', 'F-94949', 'F-14125']
# the pull-over's FSM and its four states' vtables (data, not functions)
VTABLES = [(0x820C8BB4, 16), (0x820C8CB4, 22), (0x820C8BF4, 22), (0x820C8C5C, 22), (0x820C8D0C, 22)]
TUNE = 0x8282CAC8
FLAGS = ['all_halt', 'moving', 'closing0', 'closing7', 'closing50', 'near50', 'near15', 'pick', 'gate']
# per-update flip odds of each flag
ODDS = [200, 150, 60, 60, 120, 150, 200, 30, 60]


class Rng:
    def __init__(self, seed):
        self.x = seed or 1

    def next(self):
        x = self.x
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        self.x = x
        return x

    def r(self, a, b):
        return a + self.next() % (b - a + 1)


def case_inputs(seed):
    """per update: the flags (bit k = FLAGS[k]), the cop's speed in quarters of a m/s, the target's x and z in quarters of
    a metre"""
    g = Rng(seed)
    f = [g.r(0, 1) for _ in FLAGS]
    speed, x, z = g.r(0, 60), g.r(-200, 200), g.r(-200, 200)
    out = []
    for _ in range(N):
        for k in range(len(FLAGS)):
            if g.next() % ODDS[k] == 0:
                f[k] ^= 1
        speed = max(0, min(60, speed + g.r(-2, 2)))
        x = max(-400, min(400, x + g.r(-3, 3)))
        z = max(-400, min(400, z + g.r(-3, 3)))
        out.append((sum(b << k for k, b in enumerate(f)), speed, x, z))
    return out


MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static float lf(uint32_t a){uint32_t u=load(a);float f;memcpy(&f,&u,4);return f;}
static uint8_t lb(uint32_t a){return (uint8_t)(load(a&~3u)>>((3-(a&3))*8));}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t AI=strtoul(argv[3],0,16), PL=strtoul(argv[4],0,16); std::string tok;
 while(in>>tok){ if(tok!="case") break; int N; in>>N; rt_reset(); const uint32_t M=0x40800000u, TGT=0x40A10000u, C=0x40A00000u, B=0x40A30000u, G=0x40A50000u;
  sf(TUNE+196,15.0f);
  for(int i=0;i<4;i++){ sf(0x40C00100u+4*i,1.0f); }
  Context c=fresh(); c.r3.u32=M; CTOR(c,nullptr);
  c=fresh(); c.r3.u32=M; RESET(c,nullptr);
  store(C,AI); store(C+656,3u); store(C+1076,1u); store(C+876,B); store(B+680,TGT); store(B+568,0u); store(B+688,0u); store(B+708,0u); store(load(M+76),C);
  store(TGT,PL); store(TGT+656,3u); store(M+32,TGT); store(M+36,G); store(M+20,(load(M+20)&0x00ffffffu)|0x01000000u);
  c=fresh(); c.r3.u32=M; BEGIN(c,nullptr);
  for(int k=0;k<N;k++){ uint32_t fl; int s,x,z; in>>std::hex>>fl>>std::dec>>s>>x>>z; store(0x40C00000u,fl); sf(0x40C00004u,s*0.25f);
   sf(0x40C00200u+48,0.0f); sf(0x40C00200u+52,0.0f); sf(0x40C00200u+56,0.0f); sf(0x40C00300u+48,x*0.25f); sf(0x40C00300u+52,0.0f); sf(0x40C00300u+56,z*0.25f);
   sf(0x40C00010u,1.0f); store(G+56,(fl>>8)&1?0u:1u);
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)(1.0f/60); UPDATE(c,nullptr);
   uint32_t F=load(M+160); int st=(int)load(F+12); uint32_t S=st>=0?load(load(F+4)+4*st):0;
   uint32_t a=(st==1||st==2)?load(S+16):0u, w=st==2?load(S+20):0u;
   printf("%d %d %08x %08x %08x %d %d %d %08x\n",st,(int)load(F+16),S?load(S+8):0u,a,w,(int)load(B+568),(int)lb(B+708),(int)lb(B+688),load(0x40C00010u));}
  printf("end\n");}
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--targets', nargs='*', default=[], help='F-IDs compiled in as they are')
    ap.add_argument('--more', nargs='*', default=[], help='more F-IDs to stub with a throw')
    a = ap.parse_args()
    ids = MAIN_IDS + WORLD + a.targets + a.more
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    ai, pl = rdisp.vtables_of(img, 'mcAiRacer')[0][0], rdisp.vtables_of(img, 'mcPlayer')[0][0]
    flag = lambda k: '((load(0x40C00000u)>>%d)&1)' % FLAGS.index(k)
    stubs = {name(f): 'throw std::runtime_error("unexpected call %s");' % f for f in a.more}
    stubs[name('F-90575')] = 'ctx.r3.u64=%s;' % flag('all_halt')
    stubs[name('F-96080')] = stubs[name('F-43975')] = 'ctx.r3.u64=%s;' % flag('moving')
    stubs[name('F-32902')] = ('float r=(float)ctx.f1.f64; ctx.r3.u64= r==0.0f?%s : r==7.0f?%s : r==50.0f?%s : 2;'
                              % (flag('closing0'), flag('closing7'), flag('closing50')))
    stubs[name('F-29545')] = 'float r=(float)ctx.f1.f64; ctx.r3.u64= r==50.0f?%s : r==15.0f?%s : 2;' % (flag('near50'), flag('near15'))
    stubs[name('F-14848')] = 'ctx.r3.u64=%s;' % flag('pick')
    stubs[name('F-81535')] = 'uint32_t u=load(0x40C00004u); float f; memcpy(&f,&u,4); ctx.f1.f64=(double)f;'
    stubs[name('F-35658')] = 'ctx.r3.u64 = ctx.r3.u32==0x40A00000u ? 0x40C00200u : 0x40C00300u;'
    stubs[name('F-14415')] = 'ctx.r3.u64=1;'
    stubs[name('F-36806')] = 'ctx.r3.u64=0;'
    stubs[name('F-63286')] = ''
    stubs[name('F-82234')] = 'ctx.r3.u64=0x40C00100u;'
    stubs[name('F-19124')] = ('uint32_t u=load(0x40C00010u); float p; memcpy(&p,&u,4); p*=ctx.v1.f32[0]; memcpy(&u,&p,4); store(0x40C00010u,u);')
    stubs[name('F-24671')] = ('uint32_t B=load(ctx.r3.u32+876); store(B+568,ctx.r4.u32); uint32_t q=ctx.r5.u32+16; uint8_t v=(uint8_t)(load(q&~3u)>>((3-(q&3))*8));'
                              ' auto sb=[](uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;store(a&~3u,(w&~(0xffu<<sh))|((uint32_t)b<<sh));};'
                              ' if(ctx.r4.u32==1) sb(B+688,v); if(ctx.r4.u32==5) sb(B+708,v);')
    for f in ['F-58846', 'F-72181', 'F-59773', 'F-94949', 'F-14125']:
        stubs[name(f)] = ''
    main_src = (MAIN.replace('CTOR', name('F-64611')).replace('RESET', name('F-92837')).replace('BEGIN', name('F-74547'))
                .replace('UPDATE', name('F-36876')).replace('TUNE', '0x%08Xu' % TUNE))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in MAIN_IDS], main_src, a.out, 'pullover_fsm', image=img, vtables=VTABLES,
                      targets=[addr[f] for f in a.targets], limit=800, stubs=stubs)
    seeds = [0x9E3779B9 * (i + 11) & 0xFFFFFFFF for i in range(CASES)]
    lines = []
    for seed in seeds:
        lines.append(f'case {N}')
        lines += ['%x %d %d %d' % u for u in case_inputs(seed)]
    cf = a.out / 'pullover_cases.txt'
    cf.write_text('\n'.join(lines) + '\nstop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf), '%08X' % ai, '%08X' % pl], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    outs = [o.split('\n')[:-1] for o in r.stdout.split('end\n')[:-1]]
    ref = []
    for seed, rows in zip(seeds, outs):
        ref.append(f'CASE {seed}')
        prev = None
        for k, row in enumerate(rows):
            v = row.split()
            key = v[:2] + v[5:]
            if key != prev or k % 25 == 0 or k == len(rows) - 1:
                ref.append(f'{k} {row}')
            prev = key
    a.ref.write_text('\n'.join(ref) + '\n')
    from collections import Counter
    print('cases', len(outs), 'updates', sum(map(len, outs)), 'states', dict(Counter(r.split()[0] for o in outs for r in o)), '->', a.ref)


if __name__ == '__main__':
    main()
