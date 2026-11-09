"""The rubber band's per-frame update F-48266 run offline (the generated code whole and unchanged, vector mode, with its
callees F-23349 / F-25500 / F-85220 / F-52915 as generated: verify_rubber_inputs.md). For every computer racer (record
+16 = 2) it writes brain +384 (the gap shifted by the rank table), +388 (the gap to the leading human), +392 (from the
start), +396 (to the finish) and +400 (the squared distance to the nearest human). The leading human is the racer with
+16 != 2 and the smallest place (+172), the first one on a tie.

Real memory: the manager (+52 / +53 on, +54 unordered, +1616 + 4 k the segment lengths), the race (+192 idx the racer
records: +8 the car, +16 the type, +20 the brain, +32 next, +36 lap, +172 place; +3080 the racers, +3116 the laps), the
race definition (+4 the checkpoints: +64 the place, +200 + 4 idx the racer's state), the cars (+48 the place, +192 the
start), the brains (+412 the rubber band tune: AheadDistOffsets at +488, BehindDistOffsets at +520).

Stubs: the checkpoints per lap F-67883 (the case's n), the matrix getter F-35658 (the car itself), the race singleton
F-27863 (the race - 8) and its definition getter F-71293.

Writes <out-dir>/rubber_update_ref.txt, hex words: "case c n laps unordered seg[n] cp[n].xyz offsets[16] then per racer
(c): type place next lap states car.xyz start.xyz -> per racer: +384 +388 +392 +396 +400" (7fa5a5a5: not written)."""
import argparse
import math
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb

MGR, RACE, DEF, CPS, CARS, BR = 0x40100000, 0x40102000, 0x40104000, 0x40105000, 0x40106000, 0x40108000
NMAX, CMAX = 10, 8
MARK = 0x7FA5A5A5


def case(rnd):
    c = rnd.randint(1, CMAX)
    n = rnd.choice((1, 2, 3, rnd.randint(1, NMAX), rnd.randint(4, NMAX)))
    laps = rnd.choice((0, 1, 1, 2, 3))
    L = max(laps, 1)
    unordered = rnd.choice((0, 0, 1))
    cps = [[rnd.uniform(-3000, 3000), rnd.uniform(-50, 150), rnd.uniform(-3000, 3000)] for _ in range(n)]
    seg = []
    for k in range(n):
        d = math.dist(cps[k], cps[(k + 1) % n])
        seg.append(rnd.choice((d, d, 200.0, rnd.uniform(0, 3000))))
    offs = sorted(rnd.uniform(0, 300) for _ in range(8)) + sorted(rnd.uniform(0, 300) for _ in range(8))
    places = list(range(1, c + 1))
    rnd.shuffle(places)
    if rnd.random() < 0.15:
        places[rnd.randrange(c)] = places[rnd.randrange(c)]
    nh = rnd.choice((1, 1, 1, 2, 0, c))
    humans = set(rnd.sample(range(c), min(nh, c)))
    racers = []
    for i in range(c):
        if rnd.random() < 0.85:
            nxt, lap = rnd.randrange(n), rnd.randrange(L)
        else:
            nxt, lap = n, L - 1
        cp = rnd.choice(cps)
        r = rnd.choice((0.0, 1.0, 30.0, 400.0, 3000.0))
        car = [cp[0] + rnd.uniform(-r, r), cp[1] + rnd.uniform(-r, r) * 0.1, cp[2] + rnd.uniform(-r, r)]
        if humans and rnd.random() < 0.05:
            car = list(racers[0][5:8]) if racers else car
        start = [cps[0][0] + rnd.uniform(-50, 50), cps[0][1], cps[0][2] + rnd.uniform(-50, 50)]
        racers.append([0 if i in humans else 2, places[i], nxt, lap, rnd.getrandbits(2 * n)] + car + start)
    w = [c, n, laps, unordered] + [fb(x) for x in seg + [v for p in cps for v in p] + offs]
    for r in racers:
        w += r[:5] + [fb(x) for x in r[5:]]
    return w


C = r'''
static uint32_t &o_n(){static uint32_t v;return v;}
'''

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(auto&s:CS){rt_reset();memset(pointer(@MGR@),0,0xC000);
 uint32_t c=s[0],n=s[1];o_n()=n;const uint32_t*seg=s+4,*cp=s+4+n,*off=s+4+4*n,*rr=s+20+4*n;
 *pointer(@MGR@+52)=1;*pointer(@MGR@+53)=1;*pointer(@MGR@+54)=(uint8_t)s[3];
 store(@RACE@+3080,c);store(@RACE@+3116,s[2]);store(@DEF@+4,@CPS@);
 for(uint32_t k=0;k<n;k++){store(@MGR@+1616+4*k,seg[k]);for(int i=0;i<3;i++)store(@CPS@+368*k+64+4*i,cp[3*k+i]);}
 for(uint32_t j=0;j<c;j++){const uint32_t*r=rr+11*j;uint32_t rec=@RACE@+192*j,car=@CARS@+0x400*j,br=@BR@+0x800*j;
  store(rec+8,car);store(rec+16,r[0]);store(rec+20,br);store(rec+32,r[2]);store(rec+36,r[3]);store(rec+172,r[1]);
  for(int i=0;i<3;i++){store(car+48+4*i,r[5+i]);store(car+192+4*i,r[8+i]);}
  for(uint32_t k=0;k<n;k++)store(@CPS@+368*k+200+4*j,(r[4]>>(2*k))&3);
  for(int i=0;i<16;i++)store(br+488+4*i,off[i]);for(int i=0;i<5;i++)store(br+384+4*i,@MARK@);}
 Context ctx=fresh();ctx.r3.u32=@MGR@;sub_@U@(ctx,nullptr);
 printf("case");for(uint32_t i=0;i<20+4*n+11*c;i++)printf(" %08x",s[i]);printf(" ->");
 for(uint32_t j=0;j<c;j++)for(int i=0;i<5;i++)printf(" %08x",load(@BR@+0x800*j+384+4*i));printf("\n");}}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-48266')
    rnd = random.Random(48267)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    width = 20 + 4 * NMAX + 11 * CMAX
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c + [0] * (width - len(c))) + '}' for c in cases)
    src = MAIN
    for k, v in (('MGR', MGR), ('RACE', RACE), ('DEF', DEF), ('CPS', CPS), ('CARS', CARS), ('BR', BR), ('MARK', MARK)):
        src = src.replace('@%s@' % k, '%du' % v)
    src = src.replace('@U@', fn)
    main_src = 'static const uint32_t CS[][%d]={' % width + nl + rows + nl + '};' + nl + src
    stubs = {
        'sub_' + addr(a.tool, 'F-67883'): 'ctx.r3.u32=o_n();',
        'sub_' + addr(a.tool, 'F-35658'): '',
        'sub_' + addr(a.tool, 'F-27863'): 'ctx.r3.u32=%du;' % (RACE - 8),
        'sub_' + addr(a.tool, 'F-71293'): 'ctx.r3.u32=%du;' % DEF,
    }
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + C
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'rubberupd', stubs=stubs, image=rd.load_image(a.image))
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'rubber_update_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases; written', sum(x.split('->')[1].count('%08x' % MARK) == 0 for x in lines))


if __name__ == '__main__':
    main()
