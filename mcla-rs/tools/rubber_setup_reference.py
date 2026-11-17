"""The rubber band's race set-up F-51033 (RubberBandMgr_SetupRace: F-51033(arg0, unordered, replaying)) run offline, generated
code whole and unchanged (docs/reference/ai/rubber.md 2.3):

1. odd = the difficulty (F-25301, stubbed: a table by racer handle) of the first race entry (R + 8 + 192 i, count R + 3080)
   whose type (+8) is not 0 and whose difficulty is not the race's (R + 3100).
2. step = desired <= 0 ? level - down : odd found ? (odd >= desired ? level - down : level + up) : level; the shifted ones
   clamped 0..10 (manager +56 level, +68 down, +72 up; step to +60).
3. the debug parameter (F-75950, stubbed: not set).
4. replaying: step = the replay header's +2616; else that word = step.
5. F-56464 (each AI brain's tuning copy: stubbed here).
6. the checkpoint count (F-67883, stubbed) to +1872 and the segment lengths +1616 + 4 k: |cp[k] - cp[k+1]| fused y z x,
   the last cp[n-1] - cp[0]; unordered: 200 each.

The race singleton F-27863 and the definition getter F-71293 are stubbed to the harness's memory; the three globals the
routine reads (the manager, the character manager, the replay header) are found from its own lis / lwz pairs.

Cases (seed 51033): level -2..12, desired -1..5, 0 - 8 entries of type 0 / 1 / 2 with difficulties -1..5, unordered and
replaying 0 / 1, the replay word 0..10, 0 - 12 checkpoints with random positions (some equal).
Writes <ref>: "case level desired unordered replaying rword | types.. | diffs.. | n xyz.. -> step rword count | segs.."."""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

R, M, DEF, CPS, CHM, REP, TAB = 0x40100000, 0x40110000, 0x40112000, 0x40114000, 0x40118000, 0x40119000, 0x4011A000
# racer handles: objects HND + 16 i, their vtable VT, whose slot 9 (vt+36, the name for the log line) is a harness fake
HND, VT, FAKE = 0x4011B000, 0x4011C000, 0x4011D000
STUBS = {'F-27863': 'ctx.r3.u32=R-8;', 'F-71293': 'ctx.r3.u32=DEF;', 'F-25301': 'ctx.r3.u32=load(TAB+4*((ctx.r4.u32-HND)/16));',
         'F-46414': 'ctx.r3.u32=0;', 'F-47121': '', 'F-75950': 'ctx.r3.u32=0;', 'F-56464': '',
         'F-67883': 'ctx.r3.u32=load(DEF+0x200);'}


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def globals_read(img, fn):
    """(address) of each `lis rA,hi` + later `lwz rD,lo(rA)` in the routine's 190 instructions, in order."""
    out, hi = [], {}
    for k in range(190):
        w = rd.image_u32(img, fn + 4 * k)
        op = w >> 26
        if op == 15 and (w >> 16) & 31 == 0:          # lis rD,imm (addis rD,0,imm)
            hi[(w >> 21) & 31] = (w & 0xFFFF) << 16
        elif op == 32 and (w >> 16) & 31 in hi:     # lwz rD,d(rA)
            a = hi[(w >> 16) & 31] + (((w & 0xFFFF) ^ 0x8000) - 0x8000) & 0xFFFFFFFF
            if a not in out:
                out.append(a)
            hi.pop((w >> 21) & 31, None)            # (the load overwrites its target register)
    return out


def case(rnd):
    level = rnd.choice((rnd.randint(0, 10), rnd.randint(-2, 12)))
    desired = rnd.randint(-1, 5)
    ents = [(rnd.choice((0, 1, 2, 2, 2)), rnd.choice((desired, desired, rnd.randint(-1, 5)))) for _ in range(rnd.randint(0, 8))]
    n = rnd.choice((0, 1, 2, rnd.randint(3, 12)))
    cps = []
    for _ in range(n):
        if cps and rnd.random() < 0.1:
            cps.append(cps[-1])
        else:
            cps.append((rnd.uniform(-3000, 3000), rnd.uniform(-50, 200), rnd.uniform(-3000, 3000)))
    return level, desired, rnd.getrandbits(1), rnd.getrandbits(1), rnd.randint(0, 10), ents, cps


MAIN = r'''
struct Cs{int lv,ds,un,rp,rw,ne,t[8],d[8],n;uint32_t p[12][3];};
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});int ci=0;
for(auto&s:CS){rt_reset();for(uint32_t a=R;a<FAKE;a+=4)store(a,0);
 store(GM,M);store(GC,CHM);store(GR,REP);store(M+56,(uint32_t)s.lv);store(M+68,2u);store(M+72,3u);store(M+60,0xEEEEEEEEu);
 store(REP+2616,(uint32_t)s.rw);store(R+3080,(uint32_t)s.ne);store(R+3100,(uint32_t)s.ds);
 store(VT+36,FAKE);for(int i=0;i<s.ne;i++){store(HND+16*i,VT);store(R+8+192*i,HND+16*i);store(R+8+192*i+8,(uint32_t)s.t[i]);store(TAB+4*i,(uint32_t)s.d[i]);}
 store(DEF+4,CPS);store(DEF+0x200,(uint32_t)s.n);for(int k=0;k<s.n;k++)for(int j=0;j<3;j++)store(CPS+368*k+64+4*j,s.p[k][j]);
 for(int k=0;k<12;k++)store(M+1616+4*k,0xEEEEEEEEu);
 Context c=fresh();c.r3.u32=0;c.r4.u32=s.un;c.r5.u32=s.rp;FN(c,nullptr);
 printf("case %d %d %d %d %d %d |",ci++,s.lv,s.ds,s.un,s.rp,s.rw);for(int i=0;i<s.ne;i++)printf(" %d",s.t[i]);printf(" |");
 for(int i=0;i<s.ne;i++)printf(" %d",s.d[i]);printf(" | %d",s.n);for(int k=0;k<s.n;k++)printf(" %08x %08x %08x",s.p[k][0],s.p[k][1],s.p[k][2]);
 printf(" -> %d %d %d |",(int)load(M+60),(int)load(REP+2616),(int)load(M+1872));for(int k=0;k<12;k++)printf(" %08x",load(M+1616+4*k));printf("\n");}}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    ids = ['F-51033'] + list(STUBS)
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    img = rd.load_image(a.image)
    g = globals_read(img, addr['F-51033'])
    assert len(g) == 3, [hex(x) for x in g]       # manager, character manager, replay header (order of first use)
    rnd = random.Random(51033)
    cases = [case(rnd) for _ in range(a.cases)]
    rows = []
    for lv, ds, un, rp, rw, ents, cps in cases:
        t = [e[0] for e in ents] + [0] * (8 - len(ents))
        d = [e[1] for e in ents] + [0] * (8 - len(ents))
        p = [[fb(v) for v in q] for q in cps] + [[0, 0, 0]] * (12 - len(cps))
        rows.append('{%d,%d,%d,%d,%d,%d,{%s},{%s},%d,{%s}}' % (lv, ds, un, rp, rw, len(ents), ','.join(map(str, t)),
                    ','.join(map(str, d)), len(cps), ','.join('{%du,%du,%du}' % tuple(q) for q in p)))
    consts = ''.join('static const uint32_t %s=%du;\n' % (k, v) for k, v in
                     (('R', R), ('M', M), ('DEF', DEF), ('CPS', CPS), ('CHM', CHM), ('REP', REP), ('TAB', TAB),
                      ('GM', g[0]), ('GC', g[1]), ('GR', g[2]), ('HND', HND), ('VT', VT), ('FAKE', FAKE)))
    fn = 'sub_%08X' % addr['F-51033']
    src = consts + MAIN.replace('struct Cs', 'struct Cs').replace('FN(', fn + '(')
    src = src.replace('int main(', 'static const Cs CS[]={' + ',\n'.join(rows) + '};\nint main(', 1)
    stubs = {'sub_%08X' % addr[k]: v for k, v in STUBS.items()}
    # (the stub bodies use the constants: define them ahead of the stubs)
    stubs = {k: v.replace('R-8', '0x%08Xu' % (R - 8)).replace('DEF+0x200', '0x%08Xu' % (DEF + 0x200)).replace('DEF', '0x%08Xu' % DEF)
             .replace('TAB+', '0x%08Xu+' % TAB).replace('-HND', '-0x%08Xu' % HND) for k, v in stubs.items()}
    exe = rd.build(rd.index(a.recomp), [fn], src, a.out, 'rubber_setup', image=img, stubs=stubs, fakes={FAKE: 'ctx.r3.u32=0;'}, limit=10)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, timeout=600)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout)
    print(r.stdout.count('\n'), 'cases ->', a.ref, 'globals', [hex(x) for x in g])


if __name__ == '__main__':
    main()
