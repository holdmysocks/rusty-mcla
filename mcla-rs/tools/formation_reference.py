"""Run the original's squad formations offline (static recompilation, native, flat guest memory) - the Arrow Trap's F-80532 and
the Double Pursuit's F-21534 with their states' enters (CatchUp F-81752, AlongSide F-86874) - and record every call for the
port's test (crates/game/src/testdata/formation_ref.txt; test squad_formation_matches_the_original).

The objects are the ones the code reads: the state (+20 the hysteresis, +12 its machine), the machine (+20 the tactic), the
tactic (+32 the target, +72 the cop count, +76 the cops, +164 / +168 / +172 the slots), each cop's brain at +876 (+568 the
state, +708 the side, +176 / +180 the route points the route reader F-32694 reads at the index F-68139 gives). The entities'
matrix (F-35658) is stubbed to a block at +0x100 (rows at +0 / +32, the position at +48). The brain command F-24671 is
stubbed to what its set-state does for these fields (+568 the state; for AlongSide 5 the side byte +708 from the argument's
+16); the argument blocks' ctors (F-94949, F-59773) are empty. The right-of test F-41266 and the brain state reader F-57281
run as they are.

Inputs come from an integer xorshift (the test makes the same ones): per case the tactic (even cases the Arrow Trap), the
target and cops at eighths of a metre, their walk per call, the target's yaw from a 16-entry table of unit rows (in the
header, as f32 bits), route points (none one time in five), the brains now and then set from outside, an enter at the first
call and one call in 128, dt 1/60 (one call in 8 1/30).

  ROWS <16 x (r0x r0z r2x r2z) f32 bits>
  CASE <seed>
  <sent: 0 / 1, 2 at an enter> <hysteresis bits> <slot 164> <slot 168> <slot 172> <state side> per cop
"""
import argparse
import math
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

CASES = 40
N = 150
MAIN_IDS = ['F-80532', 'F-21534', 'F-81752', 'F-86874']
STUB_IDS = ['F-35658', 'F-68139', 'F-24671', 'F-94949', 'F-59773']


def f32bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


ROWS = []
for k in range(16):
    a = k * 2 * math.pi / 16
    c, s = f32bits(math.cos(a)), f32bits(math.sin(a))
    ROWS.append((c, s, s ^ 0x80000000, c))   # row 0 (cos, 0, sin), row 2 (-sin, 0, cos)


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
    """the calls of one case: (enter, dt bits, target eighths, yaw, [(cop eighths, route eighths or None, brain override)])"""
    g = Rng(seed)
    arrow = seed % 2 == 0
    n = 3 if arrow else 2
    t = [g.r(-800, 800), g.r(0, 80), g.r(-800, 800)]
    yaw = g.r(0, 15)
    cops = [[t[0] + g.r(-320, 320), t[1] + g.r(-8, 8), t[2] + g.r(-320, 320)] for _ in range(n)]
    brains = [((0, 4, 5)[g.r(0, 2)], g.r(0, 1)) for _ in range(n)]
    calls = []
    for k in range(N):
        enter = k == 0 or g.next() % 128 == 0
        dt = f32bits(1 / 30) if g.next() % 8 == 0 else f32bits(1 / 60)
        for i in range(3):
            t[i] += g.r(-4, 4)
        if g.next() % 32 == 0:
            yaw = g.r(0, 15)
        per = []
        for c in range(n):
            for i in range(3):
                cops[c][i] += g.r(-6, 6)
            route = [cops[c][i] + g.r(-40, 40) for i in range(3)] if g.next() % 5 else None
            ov = ((0, 4, 5)[g.r(0, 2)], g.r(0, 1)) if g.next() % 64 == 0 else None
            if k == 0:
                ov = brains[c]
            per.append((list(cops[c]), route, ov))
        calls.append((enter, dt, list(t), yaw, per))
    return arrow, calls


MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
static uint8_t lb(uint32_t a){return (uint8_t)(load(a&~3u)>>((3-(a&3))*8));}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const float E=0.125f;
 while(in>>tok){ if(tok!="case") break; int arrow,n,N; in>>arrow>>n>>N; rt_reset();
  const uint32_t S=0x40800000u, F=0x40810000u, T=0x40820000u, TGT=0x40830000u, ARR=0x40840000u;
  store(S+12,F); store(F+20,T); store(T+32,TGT); store(T+36,0); store(T+72,n); store(T+76,ARR);
  store(T+164,0xffffffffu); store(T+168,0xffffffffu); store(T+172,0xffffffffu); sf(S+20,0.0f);
  for(int i=0;i<n;i++){ uint32_t C=0x40A00000u+0x4000u*i, B=C+0x2000u; store(ARR+4*i,C); store(C+876,B); store(B+176,B+0x400); store(B+180,0x00010000u);}
  for(int k=0;k<N;k++){ int en; uint32_t dtb; in>>en>>std::hex>>dtb>>std::dec; float dt; memcpy(&dt,&dtb,4);
   int tx,ty,tz,yaw; uint32_t r0x,r0z,r2x,r2z; in>>tx>>ty>>tz>>yaw>>std::hex>>r0x>>r0z>>r2x>>r2z>>std::dec;
   uint32_t M=TGT+0x100; store(M,r0x); store(M+4,0); store(M+8,r0z); store(M+32,r2x); store(M+36,0); store(M+40,r2z);
   sf(M+48,tx*E); sf(M+52,ty*E); sf(M+56,tz*E);
   for(int i=0;i<n;i++){ uint32_t C=0x40A00000u+0x4000u*i, B=C+0x2000u, CM=C+0x100; int x,y,z,has,ov; in>>x>>y>>z>>has;
    sf(CM+48,x*E); sf(CM+52,y*E); sf(CM+56,z*E);
    if(has){ int a,b,c; in>>a>>b>>c; sf(B+0x400,a*E); sf(B+0x404,b*E); sf(B+0x408,c*E); store(B+0x300,0);} else store(B+0x300,1);
    in>>ov; if(ov){ int st,sd; in>>st>>sd; store(B+568,st); sb(B+708,sd);} }
   int sent=2; Context c=fresh(); c.r3.u32=S;
   if(en){ c.f1.f64=0; if(arrow) ENTER_A(c,nullptr); else ENTER_D(c,nullptr);}
   else { c.f1.f64=(double)dt; if(arrow) FORM_A(c,nullptr); else FORM_D(c,nullptr); sent=c.r3.u32&0xff;}
   printf("%d %08x %d %d %d",sent,load(S+20),(int)load(T+164),(int)load(T+168),(int)load(T+172));
   for(int i=0;i<n;i++){ uint32_t B=0x40A00000u+0x4000u*i+0x2000u; printf(" %d %d",(int)load(B+568),(int)lb(B+708));}
   printf("\n");}
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
    ap.add_argument('--nocompile', nargs='*', default=[], help='more F-IDs to stub with a throw')
    a = ap.parse_args()
    ids = MAIN_IDS + STUB_IDS + a.nocompile
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name(f): 'throw std::runtime_error("unexpected call %s");' % f for f in a.nocompile}
    stubs[name('F-35658')] = 'ctx.r3.u64=ctx.r3.u32+0x100u;'
    stubs[name('F-68139')] = 'ctx.r3.u64=load(ctx.r3.u32+0x300u);'
    stubs[name('F-24671')] = ('uint32_t B=load(ctx.r3.u32+876); store(B+568,ctx.r4.u32); if(ctx.r4.u32==5){ uint32_t a=B+708, w=load(a&~3u);'
                              ' int sh=(3-(a&3))*8; uint8_t v=(uint8_t)(load((ctx.r5.u32+16)&~3u)>>((3-((ctx.r5.u32+16)&3))*8));'
                              ' store(a&~3u,(w&~(0xffu<<sh))|((uint32_t)v<<sh));}')
    stubs[name('F-94949')] = stubs[name('F-59773')] = ''
    main_src = (MAIN.replace('FORM_A', name('F-80532')).replace('FORM_D', name('F-21534')).replace('ENTER_A', name('F-81752'))
                .replace('ENTER_D', name('F-86874')))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in MAIN_IDS], main_src, a.out, 'formation', image=img, limit=800, stubs=stubs)
    seeds = [0x9E3779B9 * (i + 1) & 0xFFFFFFFF for i in range(CASES)]
    lines = []
    for seed in seeds:
        arrow, calls = case_inputs(seed)
        lines.append(f'case {int(arrow)} {3 if arrow else 2} {len(calls)}')
        for enter, dt, t, yaw, per in calls:
            row = ROWS[yaw]
            s = f'{int(enter)} {dt:08x} {t[0]} {t[1]} {t[2]} {yaw} ' + ' '.join('%08x' % v for v in row)
            for cop, route, ov in per:
                s += ' %d %d %d' % tuple(cop) + (' 1 %d %d %d' % tuple(route) if route else ' 0') + (' 1 %d %d' % ov if ov else ' 0')
            lines.append(s)
    cf = a.out / 'formation_cases.txt'
    cf.write_text('\n'.join(lines) + '\nstop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    outs = [o.split('\n')[:-1] for o in r.stdout.split('end\n')[:-1]]
    ref = ['ROWS ' + ' '.join('%08x' % v for row in ROWS for v in row)]
    for seed, rows in zip(seeds, outs):
        ref.append(f'CASE {seed}')
        ref += rows
    a.ref.write_text('\n'.join(ref) + '\n')
    calls = sum(map(len, outs))
    print('cases', len(outs), 'calls', calls, 'sends', sum(r.startswith('1 ') for o in outs for r in o),
          'enters', sum(r.startswith('2 ') for o in outs for r in o), '->', a.ref)


if __name__ == '__main__':
    main()
