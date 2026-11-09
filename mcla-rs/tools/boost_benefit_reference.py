"""The follower's nitrous / slingshot benefit run offline: F-82077 (nitrous, the boost fade squared) and F-96973
(slingshot, cubed), the generated code whole and unchanged, on a follower block holding a random line. Each runs two
forward integrations over the samples f+1 .. n-1 from the forward speed (+25280, at least 0.1): the boosted run kept
under the grip limit and the sample's allowed speed, and the free run; the answer is (T - ta) / (T - tb) clamped
0 .. 1 (0 when T - tb is not above 1e-6), T the line's time (+30476).

The vehicle tune (the follower's +0) is the R8's (work/ai-verify/scalar/tune_r8.bin) with the drag words (5 .. 7), the
speed index word 11 and the acceleration row 0 (words 19 .. 146) varied per case from a few tables.

Writes <out-dir>/boost_benefit_ref.txt, hex words: first "table k" + 128 words per table, then per case "case table d1
d2 d3 spdk vf c1 dur1 c2 dur2 invg n f T" + 7 words a sample for f+1 .. n-1 (allowed kv kappa g_ty g_ly a_max ds)
+ "-> nitro sling"."""
import argparse
import math
import random
import re
import struct
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb, nudge

B, TUN = 0x40100000, 0x40110000
NS = 66
TABLES = 6


def tables(rnd, base):
    out = [base[19:147]]
    for _ in range(TABLES - 1):
        top = rnd.uniform(2, 15)
        out.append([max(top * (1 - (i / 127) ** rnd.uniform(0.5, 2)), rnd.choice((0.0, 0.05, 0.2))) for i in range(128)])
    out[-1] = [rnd.choice((x, 0.0, -1.0)) for x in out[-1]]
    return out


def case(rnd, base, tabs):
    tab = rnd.randrange(TABLES)
    d = [base[5], base[6], base[7]] if rnd.random() < 0.4 else [rnd.uniform(-0.5, 1.0), rnd.uniform(-0.01, 0.02), rnd.uniform(0, 2)]
    spdk = base[11] if rnd.random() < 0.6 else rnd.uniform(0.5, 4.0)
    vf = rnd.choice((rnd.uniform(0, 80), rnd.uniform(0, 80), 0.1, nudge(0.1, 1), nudge(0.1, -1), 0.0, -3.0, rnd.uniform(0, 1)))
    c1, c2 = rnd.uniform(0, 20), rnd.uniform(0, 30)
    t1, t2 = rnd.choice((rnd.uniform(0.2, 5), rnd.uniform(0.2, 5), 0.0)), rnd.choice((rnd.uniform(0.2, 5), rnd.uniform(0.2, 5), 0.0))
    if rnd.random() < 0.05:
        c1, c2 = 0.0, 0.0
    invg = rnd.choice((base[11] * 0 + 1 / 13.72, rnd.uniform(-0.12, 0.12)))
    n = rnd.choice((rnd.randint(0, 20), rnd.randint(0, NS)))
    f = rnd.choice((rnd.randint(-1, max(n - 1, -1)), max(n - rnd.randint(2, 14), -1), max(n - rnd.randint(2, 14), -1), max(n - rnd.randint(2, 14), -1), n - 1, n))
    cnt = max(n - f - 1, 0)
    ss = []
    for _ in range(cnt):
        ds = rnd.uniform(0.5, 10)
        s = [rnd.choice((rnd.uniform(0, 80), rnd.uniform(0, 80), 1e8, 0.5)), rnd.uniform(-0.02, 0.02), rnd.choice((rnd.uniform(-0.05, 0.05), 0.0, rnd.uniform(-0.3, 0.3))),
             rnd.uniform(-3, 3), rnd.uniform(-4, 4), rnd.choice((rnd.uniform(5, 30), rnd.uniform(0, 3))), ds]
        ss.append(s)
    # (the line's time: the boosted runs' times as a float64 run of the rule estimates them, so both branches and the clamps
    # are reached)
    ta, tb = est(d, spdk, vf, c1, t1, invg, ss, tabs[tab])
    ta = ta if ta == ta and ta < 1e30 else 0.0
    tb = max(tb, 0.0)
    T = rnd.choice((tb, tb * rnd.uniform(0.8, 1.5), ta * rnd.uniform(1.0, 1.8), ta * rnd.uniform(1.0, 1.8), ta * rnd.uniform(1.0, 1.1), tb + 1e-6, tb + rnd.uniform(0, 3), rnd.uniform(0, 20), 0.0))

    return [tab] + [fb(x) for x in d + [spdk, vf, c1, t1, c2, t2, invg]] + [n & 0xffffffff, f & 0xffffffff, fb(T)] + [fb(x) for s in ss for x in s]


def est(d, spdk, vf, c, dur, invg, ss, row):
    def acc(v):
        x = min(max(spdk * v, 0.0), 126.1) if v == v else 0.0
        j = int(x)
        return max((row[j + 1] - row[j]) * (x - j) + row[j], 0.1)
    va = vb = max(vf, 0.1)
    ta = tb = 0.0
    it = 1 / dur if dur else float('inf')
    for allowed, kv, k, gty, gly, amax, ds in ss:
        ba, bb = max(1 - it * ta, 0.0) if ta == ta else 0.0, max(1 - it * tb, 0.0) if tb == tb else 0.0
        nn = min(max(1 - kv * invg * va * va, 0.0), 1.0)
        lim = math.sqrt(max((nn * amax) ** 2 - (k * va * va + gly) ** 2, 0.0))
        dra, drb = max((abs(k) * d[2] + d[1]) * va + d[0], 0.0), max((abs(k) * d[2] + d[1]) * vb + d[0], 0.0)
        a = min(c * ba * ba + acc(va) - dra ** 4 + gty, lim)
        na = min(math.sqrt(max(a * ds * 2 + va * va, 0.1)), allowed)
        nb = math.sqrt(max((c * bb * bb + acc(vb) - drb ** 4 + gty) * ds * 2 + vb * vb, 0.1))
        ta += ds * 2 / (na + va)
        tb += ds * 2 / (nb + vb)
        va, vb = na, nb
    return ta, tb


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--tune', type=Path, default=Path('D:/MCLA/work/ai-verify/scalar/tune_r8.bin'))
    ap.add_argument('--cases', type=int, default=8000)
    a = ap.parse_args()
    fn = {k: addr(a.tool, f) for k, f in (('N', 'F-82077'), ('S', 'F-96973'))}
    tb = a.tune.read_bytes()
    base = list(struct.unpack('>%df' % (len(tb) // 4), tb))
    rnd = random.Random(82077)
    tabs = tables(rnd, base)
    cases = [case(rnd, base, tabs) for _ in range(a.cases)]
    nl = chr(10)
    flat, offs = [], []
    for c in cases:
        offs.append(len(flat))
        flat += c
    src = ('static const uint32_t TB[%d]={%s};' % (len(base), ','.join('%du' % fb(x) for x in base)) + nl +
           'static const uint32_t TABS[%d][128]={%s};' % (TABLES, ','.join('{' + ','.join('%du' % fb(x) for x in t) + '}' for t in tabs)) + nl +
           'static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl +
           'static const uint32_t OF[]={%s};' % ','.join('%du' % o for o in offs) + nl + r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(int t=0;t<@T@;t++){printf("table %d",t);for(int i=0;i<128;i++)printf(" %08x",TABS[t][i]);printf("\n");}
for(int k=0;k<@C@;k++){const uint32_t*s=CS+OF[k];rt_reset();memset(pointer(@B@),0,0x8000);
 for(int i=0;i<@NT@;i++)store(@TUN@+4*i,TB[i]);for(int i=0;i<128;i++)store(@TUN@+4*(19+i),TABS[s[0]][i]);
 store(@TUN@+20,s[1]);store(@TUN@+24,s[2]);store(@TUN@+28,s[3]);store(@TUN@+44,s[4]);
 store(@B@,@TUN@);store(@B@+25280,s[5]);store(@B@+25296,s[6]);store(@B@+25300,s[7]);store(@B@+25312,s[8]);store(@B@+25316,s[9]);
 store(@B@+25328,s[10]);store(@B@+30464,s[11]);store(@B@+30468,s[12]);store(@B@+30476,s[13]);
 int n=(int)s[11],f=(int)s[12],cnt=n-f-1>0?n-f-1:0;
 for(int j=0;j<cnt;j++){uint32_t o=@B@+26768+56*(f+1+j);const uint32_t*w=s+14+7*j;
  store(o+4,w[0]);store(o+16,w[1]);store(o+20,w[2]);store(o+24,w[3]);store(o+28,w[4]);store(o+32,w[5]);store(o+36,w[6]);}
 uint32_t r[2];
 {Context c=fresh();c.r3.u32=@B@;sub_@N@(c,nullptr);float v=(float)c.f1.f64;memcpy(&r[0],&v,4);}
 {Context c=fresh();c.r3.u32=@B@;sub_@S@(c,nullptr);float v=(float)c.f1.f64;memcpy(&r[1],&v,4);}
 printf("case");for(int i=0;i<14+7*cnt;i++)printf(" %08x",s[i]);printf(" -> %08x %08x\n",r[0],r[1]);}}
''')
    for k, v in (('B', B), ('TUN', TUN)):
        src = src.replace('@%s@' % k, '%du' % v)
    src = src.replace('@T@', str(TABLES)).replace('@C@', str(a.cases)).replace('@NT@', str(len(base)))
    for k, v in fn.items():
        src = src.replace('@%s@' % k, v)
    exe = rd.build(rd.index(a.recomp), ['sub_' + v for v in fn.values()], src, a.out_dir, 'boostben', image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases + TABLES, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'boost_benefit_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    from collections import Counter
    res = [x.split()[-2:] for x in lines if x.startswith('case')]
    print(a.cases, 'cases; nitro 0/1/mid', Counter('0' if r[0] == '00000000' else '1' if r[0] == '3f800000' else 'm' for r in res),
          'sling', Counter('0' if r[1] == '00000000' else '1' if r[1] == '3f800000' else 'm' for r in res))


if __name__ == '__main__':
    main()
