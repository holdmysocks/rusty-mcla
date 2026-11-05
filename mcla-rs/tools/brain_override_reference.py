"""Run the original's police brain-state ticks offline (static recompilation, native, flat guest memory) and record the
overrides they put over the driver's input, for the port's test (crates/game/src/testdata/brain_override_ref.txt; test
brain_override_matches_the_original).

The ticks run unchanged on a brain object in guest memory: Pursuit F-68176, AlongSide F-33668 and Stopping's apply F-87827
(the Stopping tick F-10174 is it when the brain has a tactic, +676). The four setters the ticks call (throttle
F-50239 +744, brake F-39048 +748, handbrake F-10965 +752, steer F-77791 +756, each clamped) run too; the fields are reset to
-1 / -1 / -1 / -1000 (no override) before each tick. The world is stubbed: the target moving F-43975, ahead F-99390, the same
direction F-80249, the brain's race flag F-42951, the cop's and the target's speed (F-81535, F-41260) and their matrices
(F-38673, F-14038; the position at +48). The tune's fSafeDist (+192) is 30. Inputs per sample: the four flags, the speeds in
quarters of a m/s, the cop's position and the target's offset in quarters of a metre (offsets often on the 20 m / 30 m /
1.5 x and 1 + side-distance spheres), the side distance +712, and the Stop parameters +688 / +692 / +696 / +700.

  <flags hex> <cop speed q> <target speed q> <cx> <cy> <cz> <ox> <oy> <oz> <+712 bits> <+688> <+692 bits> <+696 bits> <+700 bits>
    | <Pursuit +744 +748 +752 +756 bits> | <AlongSide ...> | <Stopping ...>
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import struct
import recomp_dispatch as rdisp

SAMPLES = 4000
MAIN_IDS = ['F-68176', 'F-33668', 'F-87827']
WORLD = ['F-43975', 'F-99390', 'F-80249', 'F-42951', 'F-81535', 'F-41260', 'F-38673', 'F-14038']
SETTERS = ['F-50239', 'F-39048', 'F-10965', 'F-77791']
TUNE = 0x8282CAC8
FLAGS = ['moving', 'ahead', 'same_dir', 'ed78']


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def samples():
    g = random.Random(20261105)
    out = []
    for _ in range(SAMPLES):
        fl = g.randrange(16)
        cs, ts = g.randrange(0, 61), g.randrange(0, 61)
        if g.random() < 0.3:
            ts = cs  # equal speeds: the strict "faster" test
        side = g.choice([3.0, 4.0, 5.0, 6.5, 8.0])
        cx, cy, cz = g.randrange(-800, 801), g.randrange(-20, 21), g.randrange(-800, 801)
        r = g.choice([20.0, 30.0, 1.5 * side, side + 1.0, None])
        if r is None:
            ox, oy, oz = g.randrange(-180, 181), g.randrange(-8, 9), g.randrange(-180, 181)
        else:
            # on or about the sphere: an axis offset of r (exact squares) or a random direction scaled to r
            q = int(round(r * 4)) + g.randrange(-1, 2)
            if g.random() < 0.5:
                ox, oy, oz = g.choice([(q, 0, 0), (0, 0, q), (-q, 0, 0), (0, 0, -q)])
            else:
                import math
                a = g.random() * 6.283185
                ox, oy, oz = int(round(q * math.cos(a))), g.randrange(-2, 3), int(round(q * math.sin(a)))
        hb = g.randrange(2)
        amt = g.choice([1.0, 0.5, 1.5, 0.0, -0.5])
        thr = g.choice([-1.0, 0.0, 0.25, 1.2])
        st = g.choice([-1000.0, -1.0, 1.0, 0.3, 2.0, -3.0])
        out.append((fl, cs, ts, cx, cy, cz, ox, oy, oz, fb(side), hb, fb(amt), fb(thr), fb(st)))
    return out


MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;store(a&~3u,(w&~(0xffu<<sh))|((uint32_t)b<<sh));}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); rt_reset(); const uint32_t B=0x40A30000u, S=0x40A50000u, F=0x40A60000u;
 sf(TUNE+192,30.0f); store(S+12,F); store(F+20,B); store(B+676,1u);
 uint32_t fl; int cs,ts,cx,cy,cz,ox,oy,oz; uint32_t side,hb,amt,thr,st;
 while(in>>std::hex>>fl>>std::dec>>cs>>ts>>cx>>cy>>cz>>ox>>oy>>oz>>std::hex>>side>>hb>>amt>>thr>>st>>std::dec){
  store(0x40C00000u,fl); sf(0x40C00004u,cs*0.25f); sf(0x40C00008u,ts*0.25f);
  sf(0x40C00200u+48,cx*0.25f); sf(0x40C00200u+52,cy*0.25f); sf(0x40C00200u+56,cz*0.25f);
  sf(0x40C00300u+48,(cx+ox)*0.25f); sf(0x40C00300u+52,(cy+oy)*0.25f); sf(0x40C00300u+56,(cz+oz)*0.25f);
  store(B+712,side); sb(B+688,(uint8_t)hb); store(B+692,amt); store(B+696,thr); store(B+700,st);
  const uint32_t TICK[3]={0,1,2};
  for(int k=0;k<3;k++){ sf(B+744,-1.0f); sf(B+748,-1.0f); sf(B+752,-1.0f); sf(B+756,-1000.0f);
   Context c=fresh(); c.r3.u32=S; if(k==0) PURSUIT(c,nullptr); else if(k==1) ALONG(c,nullptr); else STOPPING(c,nullptr);
   printf("%08x %08x %08x %08x%s",load(B+744),load(B+748),load(B+752),load(B+756),k==2?"\n":" | "); (void)TICK; }
 }
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
    ids = MAIN_IDS + WORLD + SETTERS + a.more
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    flag = lambda k: '((load(0x40C00000u)>>%d)&1)' % FLAGS.index(k)
    ldf = lambda at: 'uint32_t u=load(%su); float f; memcpy(&f,&u,4); ctx.f1.f64=(double)f;' % at
    stubs = {name(f): 'throw std::runtime_error("unexpected call %s");' % f for f in a.more}
    stubs[name('F-43975')] = 'ctx.r3.u64=%s;' % flag('moving')
    stubs[name('F-99390')] = 'ctx.r3.u64=%s;' % flag('ahead')
    stubs[name('F-80249')] = 'ctx.r3.u64=%s;' % flag('same_dir')
    stubs[name('F-42951')] = 'ctx.r3.u64=%s;' % flag('ed78')
    stubs[name('F-81535')] = ldf('0x40C00004')
    stubs[name('F-41260')] = ldf('0x40C00008')
    stubs[name('F-38673')] = 'ctx.r3.u64=0x40C00200u;'
    stubs[name('F-14038')] = 'ctx.r3.u64=0x40C00300u;'
    main_src = (MAIN.replace('PURSUIT', name('F-68176')).replace('ALONG', name('F-33668')).replace('STOPPING', name('F-87827'))
                .replace('TUNE', '0x%08Xu' % TUNE))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in MAIN_IDS + SETTERS], main_src, a.out, 'brain_override', image=img,
                      vtables=[], targets=[], limit=800, stubs=stubs)
    ins = samples()
    cf = a.out / 'brain_cases.txt'
    cf.write_text('\n'.join('%x %d %d %d %d %d %d %d %d %08x %d %08x %08x %08x' % s for s in ins) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    outs = r.stdout.splitlines()
    assert len(outs) == len(ins), (len(outs), len(ins))
    lines = ['%x %d %d %d %d %d %d %d %d %08x %d %08x %08x %08x | %s' % (*s, o) for s, o in zip(ins, outs)]
    a.ref.write_text('\n'.join(lines) + '\n')
    nb = [sum(1 for o in outs if o.split(' | ')[k].split()[1] != 'bf800000') for k in range(2)]
    print('samples', len(outs), 'brake by Pursuit', nb[0], 'by AlongSide', nb[1], '->', a.ref)


if __name__ == '__main__':
    main()
