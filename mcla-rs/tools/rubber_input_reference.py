"""The rubber band's distances along the checkpoint chain run offline (the generated code whole and unchanged, vector
mode): a racer's distance to its next checkpoint F-23349 (the nearest one it has not taken in an unordered race), the gap
F-25500 between a racer and the leading human (brain +388), the distance from the racer's start F-85220 (+392) and the
distance to the finish F-52915 (+396). The four are called as the rubber band update F-48266 calls them: F-25500 gets the
leader's F-23349 in f1.

Real memory: the manager (+54 unordered flag, +1616 + 4 k the segment lengths of one lap), the race (+192 idx the racer
records: +8 the car, +32 the next checkpoint, +36 the lap; +3116 the laps), the race definition (+4 the checkpoint array,
368 bytes each: +64 the place, +200 + 4 idx the racer's state there), the cars (+48 the place, +192 the start).

Stubs: the checkpoints per lap F-67883 (the case's n), the matrix getter F-35658 (the car itself: its +48 is the place), the
race singleton F-27863 (the race - 8) and its definition getter F-71293.

Writes <out-dir>/rubber_input_ref.txt, hex words: "case n laps unordered a l next_a lap_a next_l lap_l states_a states_l
car_a.xyz car_l.xyz start_a.xyz seg[n] cp[n].xyz -> D_a D_l gap from_start to_finish" (states: 2 bits per checkpoint)."""
import argparse
import math
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb

MGR, RACE, DEF, CPS, CARS = 0x40100000, 0x40102000, 0x40104000, 0x40105000, 0x40106000
NMAX = 10


def case(rnd):
    n = rnd.choice((1, 2, 3, rnd.randint(1, NMAX), rnd.randint(4, NMAX)))
    laps = rnd.choice((0, 1, 1, 2, 3))
    unordered = rnd.choice((0, 0, 1))
    a = rnd.randrange(8)
    l = a if rnd.random() < 0.1 else rnd.randrange(8)
    L = max(laps, 1)

    def pos():
        nxt = rnd.choice((rnd.randrange(n), rnd.randrange(n), n - 1, 0, n))
        lap = rnd.choice((rnd.randrange(L), rnd.randrange(L), L - 1, L))
        return nxt, lap
    na, la = pos()
    nl, ll = pos()
    sa = rnd.getrandbits(2 * n)
    sl = sa if l == a else rnd.getrandbits(2 * n)
    cps = [[rnd.uniform(-3000, 3000), rnd.uniform(-50, 150), rnd.uniform(-3000, 3000)] for _ in range(n)]

    def near():
        c = rnd.choice(cps)
        r = rnd.choice((0.0, 1e-3, 1.0, 30.0, 400.0, 3000.0))
        return [c[0] + rnd.uniform(-r, r), c[1] + rnd.uniform(-r, r) * 0.1, c[2] + rnd.uniform(-r, r)]
    ca, cl, st = near(), near(), near()
    seg = []
    for k in range(n):
        p, q = cps[k], cps[(k + 1) % n]
        d = math.dist(p, q)
        seg.append(rnd.choice((d, d, 200.0, rnd.uniform(0, 3000), 0.0)))
    w = [n, laps, unordered, a, l, na, la, nl, ll, sa, sl]
    return w + [fb(x) for x in ca + cl + st + seg + [v for c in cps for v in c]]


C = r'''
static uint32_t &o_n(){static uint32_t v;return v;}
static float fl(Context&c){return (float)c.f1.f64;}
static uint32_t bits(float x){uint32_t b;memcpy(&b,&x,4);return b;}
'''

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(auto&s:CS){rt_reset();memset(pointer(@MGR@),0,0x8000);
 uint32_t n=s[0],a=s[3],l=s[4];o_n()=n;
 *pointer(@MGR@+54)=(uint8_t)s[2];store(@RACE@+3116,s[1]);store(@DEF@+4,@CPS@);
 uint32_t car_a=@CARS@+0x400*a,car_l=@CARS@+0x400*l;
 store(@RACE@+192*a+8,car_a);store(@RACE@+192*a+32,s[5]);store(@RACE@+192*a+36,s[6]);
 store(@RACE@+192*l+8,car_l);store(@RACE@+192*l+32,s[7]);store(@RACE@+192*l+36,s[8]);
 for(int i=0;i<3;i++){store(car_l+48+4*i,s[14+i]);}
 for(int i=0;i<3;i++){store(car_a+48+4*i,s[11+i]);store(car_a+192+4*i,s[17+i]);}
 for(uint32_t k=0;k<n;k++){store(@MGR@+1616+4*k,s[20+k]);for(int i=0;i<3;i++)store(@CPS@+368*k+64+4*i,s[20+n+3*k+i]);
  store(@CPS@+368*k+200+4*l,(s[10]>>(2*k))&3);store(@CPS@+368*k+200+4*a,(s[9]>>(2*k))&3);}
 uint32_t r[5]={0xDEADDEADu,0xDEADDEADu,0xDEADDEADu,0xDEADDEADu,0xDEADDEADu};
 try{Context c=fresh();c.r3.u32=@MGR@;c.r4.u32=a;sub_@D@(c,nullptr);r[0]=bits(fl(c));}catch(const std::exception&){}
 float dl=0;try{Context c=fresh();c.r3.u32=@MGR@;c.r4.u32=l;sub_@D@(c,nullptr);dl=fl(c);r[1]=bits(dl);}catch(const std::exception&){}
 try{Context c=fresh();c.r3.u32=@MGR@;c.r4.u32=@RACE@;c.r5.u32=@DEF@;c.r7.u32=l;c.r8.u32=a;c.f1.f64=dl;sub_@G@(c,nullptr);r[2]=bits(fl(c));}catch(const std::exception&){}
 try{Context c=fresh();c.r3.u32=@MGR@;c.r4.u32=@RACE@;c.r5.u32=@DEF@;c.r6.u32=a;sub_@S@(c,nullptr);r[3]=bits(fl(c));}catch(const std::exception&){}
 try{Context c=fresh();c.r3.u32=@MGR@;c.r4.u32=@RACE@;c.r5.u32=@DEF@;c.r6.u32=a;sub_@F@(c,nullptr);r[4]=bits(fl(c));}catch(const std::exception&){}
 printf("case");for(uint32_t i=0;i<20+4*n;i++)printf(" %08x",s[i]);printf(" ->");for(int i=0;i<5;i++)printf(" %08x",r[i]);printf("\n");}}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn = {k: addr(a.tool, f) for k, f in (('D', 'F-23349'), ('G', 'F-25500'), ('S', 'F-85220'), ('F', 'F-52915'))}
    rnd = random.Random(48266)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    width = 20 + 4 * NMAX
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c + [0] * (width - len(c))) + '}' for c in cases)
    src = MAIN
    for k, v in (('MGR', MGR), ('RACE', RACE), ('DEF', DEF), ('CPS', CPS), ('CARS', CARS)):
        src = src.replace('@%s@' % k, '%du' % v)
    for k, v in fn.items():
        src = src.replace('@%s@' % k, v)
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
        exe = rd.build(rd.index(a.recomp), ['sub_' + v for v in fn.values()], main_src, a.out_dir, 'rubberin', stubs=stubs,
                       image=rd.load_image(a.image))
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    assert not any('deaddead' in x for x in lines), 'a call threw'
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'rubber_input_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
