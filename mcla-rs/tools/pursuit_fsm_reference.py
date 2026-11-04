"""Run the original's Pursuit tactic offline (static recompilation, native, flat guest memory) and record its machine for the
port's test (crates/game/src/testdata/pursuit_fsm_ref.txt; test pursuit_machine_matches_the_original).

The tactic object is built as the game builds it: ctor F-55508 (its FSM and the states Delay, CatchUp, AlongSide, Block,
Arrest, Halt, Search), reset F-20606, its cop and the target set as the engage sets them (+76, +32, active +20), the Delay's
hold +164 0, begin F-89627, then the tactic update F-68032 with dt = 1/60 s, N times; the FSM's code and the states' enters
and ticks run unchanged. The global tune gets the disc's values for what the ticks read (fSafeDist 30, fLostTgtThresholdTime
1.5, fAlongSideDist 5, fArrestTgtThresholdSpd 10). The world is per-update inputs: the proximity test F-29545 by its radius
(5 m, 1.5 x fAlongSideDist, else fSafeDist), target moving F-96080, same way F-80249, ahead F-99390, on the right F-30098,
facing the same way F-43223, every cop within F-90575, closing F-32902, the side pick F-14848, the bearing F-90304, the
target's speed F-41260 (the base update F-63286 keeps it at +64 as it does), the re-pick gate (tactic +36 -> +56). The cop
is valid (F-14415), the arrest test F-36806 false and the brain never loses the target (+773 0): Arrest and Search are the
port's heat. The brain command F-24671 is stubbed to what its set-state does for these fields (+568 the state; Stop 1 the
handbrake byte +688, AlongSide 5 the side +708, from the argument's +16); the argument ctors and the steer F-77791 are empty.

Inputs come from an integer xorshift (the test makes the same ones): the flags as random walks, the bearing in whole degrees,
the speed in quarters of a m/s with now and then a hard stop.

  CASE <seed>
  <k> <state> <asked> <time bits> <+16 bits> <+20 bits> <brain +568> <+708> <+688>      the updates where the state, the
  asked state or the brain changed, every 25th and the last
  (state: 1 CatchUp, 2 AlongSide, 3 Block, 5 Halt; +16 / +20 the AlongSide's out-of-reach and side timers, Halt's steer in
  +16; 0 in other states)
"""
import argparse
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

CASES = 40
N = 1200
NOCOMPILE = ['F-35685', 'F-75034', 'F-29294', 'F-38446', 'F-62425', 'F-29290', 'F-33229', 'F-62311']
MAIN_IDS = ['F-55508', 'F-20606', 'F-89627', 'F-68032']
WORLD = ['F-29545', 'F-96080', 'F-80249', 'F-99390', 'F-30098', 'F-43223', 'F-90575', 'F-32902', 'F-14848', 'F-90304', 'F-41260',
         'F-14415', 'F-36806', 'F-63286', 'F-24671', 'F-77791', 'F-72181', 'F-59773', 'F-94949']
TARGETS = ['F-87882', 'F-54202', 'F-77220', 'F-49619', 'F-47121', 'F-74264', 'F-74492', 'F-24596', 'F-37056', 'F-82134', 'F-29906',
           'F-33778', 'F-94002']
# the Pursuit's FSM and its seven states' vtables (data, not functions)
VTABLES = [(0x820C8764, 16), (0x820C8904, 22), (0x820C8964, 22), (0x820C87A4, 22), (0x820C89BC, 22), (0x820C87FC, 22),
           (0x820C8854, 22), (0x820C88AC, 22)]
TUNE = 0x8282CAC8
FLAGS = ['near_safe', 'near_along', 'near5', 'moving', 'same_dir', 'ahead', 'on_right', 'same_facing', 'all_block', 'closing',
         'pick', 'gate']
# per-update flip odds of each flag (the slow ones: within range, moving, the same way, ahead)
ODDS = [300, 150, 60, 300, 300, 300, 30, 30, 40, 40, 30, 60]


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
    """per update: the 12 flags (bit k = FLAGS[k]), the bearing in degrees, the speed in quarters of a m/s"""
    g = Rng(seed)
    f = [g.r(0, 1) for _ in FLAGS]
    bearing, speed = g.r(60, 120), g.r(0, 80)
    out = []
    for _ in range(N):
        for k in range(len(FLAGS)):
            if g.next() % ODDS[k] == 0:
                f[k] ^= 1
        bearing = max(50, min(130, bearing + g.r(-3, 3)))
        speed = max(0, min(120, speed + g.r(-2, 2)))
        if g.next() % 97 == 0:
            speed = max(0, speed - g.r(10, 40))
        out.append((sum(b << k for k, b in enumerate(f)), bearing, speed))
    return out


MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
static uint8_t lb(uint32_t a){return (uint8_t)(load(a&~3u)>>((3-(a&3))*8));}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t AI=strtoul(argv[3],0,16), PL=strtoul(argv[4],0,16); std::string tok;
 while(in>>tok){ if(tok!="case") break; int N; in>>N; rt_reset(); const uint32_t M=0x40800000u, TGT=0x40A10000u, C=0x40A00000u, B=0x40A30000u, G=0x40A50000u;
  sf(TUNE+192,30.0f); sf(TUNE+212,1.5f); sf(TUNE+304,5.0f); sf(TUNE+368,10.0f);
  Context c=fresh(); c.r3.u32=M; CTOR(c,nullptr);
  c=fresh(); c.r3.u32=M; RESET(c,nullptr);
  store(C,AI); store(C+656,3u); store(C+1076,1u); store(C+876,B); store(B+680,TGT); store(B+568,0u); store(B+688,0u); store(B+708,0u); store(load(M+76),C);
  store(TGT,PL); store(TGT+656,3u); store(M+32,TGT); store(M+36,G); store(M+20,(load(M+20)&0x00ffffffu)|0x01000000u); sf(M+164,0.0f);
  c=fresh(); c.r3.u32=M; BEGIN(c,nullptr);
  for(int k=0;k<N;k++){ uint32_t fl; int b,s; in>>std::hex>>fl>>std::dec>>b>>s; store(0x40C00000u,fl); sf(0x40C00004u,(float)b); sf(0x40C00008u,s*0.25f); store(G+56,(fl>>11)&1?0u:1u);
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)(1.0f/60); UPDATE(c,nullptr);
   uint32_t F=load(M+160); int st=(int)load(F+12); uint32_t S=st>=0?load(load(F+4)+4*st):0;
   uint32_t a=(st==2||st==5)?load(S+16):0u, z=st==2?load(S+20):0u;
   printf("%d %d %08x %08x %08x %d %d %d\n",st,(int)load(F+16),S?load(S+8):0u,a,z,(int)load(B+568),(int)lb(B+708),(int)lb(B+688));}
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
    ap.add_argument('--more', nargs='*', default=[], help='more F-IDs to stub with a throw')
    a = ap.parse_args()
    ids = MAIN_IDS + NOCOMPILE + WORLD + TARGETS + a.more
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    ai, pl = rdisp.vtables_of(img, 'mcAiRacer')[0][0], rdisp.vtables_of(img, 'mcPlayer')[0][0]
    flag = lambda k: '((load(0x40C00000u)>>%d)&1)' % FLAGS.index(k)
    f32 = lambda a: '([](uint32_t u){float f;memcpy(&f,&u,4);return f;})(load(%s))' % a
    stubs = {name(f): 'throw std::runtime_error("unexpected call %s");' % f for f in NOCOMPILE + a.more}
    stubs[name('F-29545')] = ('float r=(float)ctx.f1.f64; ctx.r3.u64= r==5.0f?%s : r==7.5f?%s : %s;'
                              % (flag('near5'), flag('near_along'), flag('near_safe')))
    for f, k in [('F-96080', 'moving'), ('F-80249', 'same_dir'), ('F-99390', 'ahead'), ('F-30098', 'on_right'), ('F-43223', 'same_facing'),
                 ('F-90575', 'all_block'), ('F-32902', 'closing'), ('F-14848', 'pick')]:
        stubs[name(f)] = 'ctx.r3.u64=%s;' % flag(k)
    stubs[name('F-90304')] = 'ctx.f1.f64=(double)%s;' % f32('0x40C00004u')
    stubs[name('F-41260')] = 'ctx.f1.f64=(double)%s;' % f32('0x40C00008u')
    stubs[name('F-14415')] = 'ctx.r3.u64=1;'
    stubs[name('F-36806')] = 'ctx.r3.u64=0;'
    stubs[name('F-63286')] = 'store(ctx.r3.u32+64,load(0x40C00008u));'
    stubs[name('F-24671')] = ('uint32_t B=load(ctx.r3.u32+876); store(B+568,ctx.r4.u32); uint32_t q=ctx.r5.u32+16; uint8_t v=(uint8_t)(load(q&~3u)>>((3-(q&3))*8));'
                              ' auto sb=[](uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;store(a&~3u,(w&~(0xffu<<sh))|((uint32_t)b<<sh));};'
                              ' if(ctx.r4.u32==1) sb(B+688,v); if(ctx.r4.u32==5) sb(B+708,v);')
    for f in ['F-77791', 'F-72181', 'F-59773', 'F-94949']:
        stubs[name(f)] = ''
    main_src = (MAIN.replace('CTOR', name('F-55508')).replace('RESET', name('F-20606')).replace('BEGIN', name('F-89627'))
                .replace('UPDATE', name('F-68032')).replace('TUNE', '0x%08Xu' % TUNE))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in MAIN_IDS], main_src, a.out, 'pursuit_fsm', image=img, vtables=VTABLES,
                      targets=[addr[f] for f in TARGETS], limit=800, stubs=stubs)
    seeds = [0x9E3779B9 * (i + 7) & 0xFFFFFFFF for i in range(CASES)]
    lines = []
    for seed in seeds:
        lines.append(f'case {N}')
        lines += ['%x %d %d' % u for u in case_inputs(seed)]
    cf = a.out / 'pursuit_cases.txt'
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
