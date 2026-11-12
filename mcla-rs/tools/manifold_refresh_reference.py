"""The manifold refresh F-26862 run offline on the generated code, with its callees unchanged (the contact refresh F-33100
and the record copy F-86309): a manifold of 1 .. 6 kept contacts against the city (the second matrix the identity), the
body's matrix as it stands. Rule: docs/reference/vehicle/manifold_refresh_check.md.

Memory: manifold 0x1000 (+1260 the count, +1264 the selected slot, the 176-byte records from +192), the body's matrix
0x1800, the city's (identity) 0x1840. The image's globals the routine reads: the age bound (set as the game's simulator
sets it, 1: F-51097), the start vector 0 and the masks as the running game has them (x y z all ones, w 0; the fill
x y z 0, w 1.0: zero in the image, as broad_reference.py sets them); -0.04 and the 2 cm bound read from the image.

Writes <out-dir>/manifold_refresh_ref.txt, one case a line:
  "M n m0..m15 | (age depth l.x l.y l.z w.x w.y w.z b.x b.y b.z tag ob.x ob.y ob.z)*n -> n' (tag age depth ob.x ob.y ob.z t.x t.y t.z)*n'"
(hex words as x + 8 hex digits; `w` the city's point (+32 = +64), `b` the back normal (+96, the normal's opposite), `tag`
the record's number before, kept in its word +160, which the copy carries; `ob` the body's point +48 as it was, `t` the
target +144 after, set to 0.5 before). Test
`physics::hull::tests::manifold_refresh_matches_the_original`."""
import argparse
import math
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)
import artic_reference  # noqa: F401 (its shim: the SDK helpers the vector code needs)

fb, addr = bs.fb, bs.addr
MAXN = 6
RW = 12  # input words a record


def rot(rnd):
    a, b, c = (rnd.uniform(-math.pi, math.pi) for _ in range(3))
    ca, sa, cb, sb, cc, sc = math.cos(a), math.sin(a), math.cos(b), math.sin(b), math.cos(c), math.sin(c)
    rz = [[ca, sa, 0], [-sa, ca, 0], [0, 0, 1]]
    ry = [[cb, 0, -sb], [0, 1, 0], [sb, 0, cb]]
    rx = [[1, 0, 0], [0, cc, sc], [0, -sc, cc]]
    mul = lambda p, q: [[sum(p[i][k] * q[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    return mul(mul(rz, ry), rx)


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if 0.1 < n <= 1:
            return [x / n for x in v]


def case(rnd):
    r = rot(rnd)
    at = [rnd.uniform(-3000, 3000), rnd.uniform(-50, 50), rnd.uniform(-3000, 3000)]
    m = [r[0] + [0.0], r[1] + [0.0], r[2] + [0.0], at + [1.0]]
    n = rnd.randrange(1, MAXN + 1)
    recs = []
    for k in range(n):
        age = rnd.choice([0, 0, 1, 1, 2, 3, 7])
        l = [rnd.uniform(-2.5, 2.5) for _ in range(3)]
        ob = [at[i] + sum(l[j] * m[j][i] for j in range(3)) for i in range(3)]
        back = unit(rnd)
        d = rnd.choice([rnd.uniform(-0.08, 0.05), rnd.uniform(-0.045, -0.035), -0.04, 0.0])
        tang = unit(rnd)
        t = [tang[i] - back[i] * sum(tang[j] * back[j] for j in range(3)) for i in range(3)]
        s = rnd.choice([0.0, rnd.uniform(0, 0.04), rnd.uniform(0.018, 0.022), 0.02])
        w = [ob[i] - back[i] * d + t[i] * s for i in range(3)]
        if rnd.random() < 0.1:
            w = [w[i] + rnd.uniform(-0.5, 0.5) for i in range(3)]
        depth = rnd.uniform(-0.1, 0.1) if age else d
        if age == 0 and rnd.random() < 0.5:
            ob = [ob[i] + rnd.uniform(-0.03, 0.03) for i in range(3)]
        recs.append([age, depth] + l + w + back + [k] + ob)
    return m, recs


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-26862')
    rnd = random.Random(26862)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    flat = []
    for m, recs in cases:
        w = [len(recs)] + [fb(x) for row in m for x in row]
        for r in recs:
            w += [r[0], fb(r[1])] + [fb(x) for x in r[2:11]] + [r[11]] + [fb(x) for x in r[12:15]]
        w += [0] * ((RW + 3) * (MAXN - len(recs)))
        flat.append('{' + ','.join('0x%x' % x for x in w) + '}')
    setup = ('for(uint32_t q=0x1000;q<0x2000;q+=4)store(q,0);uint32_t n=c[0];store(0x828127F0,1);for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);store(0x8286093C,0x3F800000);for(int k=0;k<4;k++)store(0x828608E0+4*k,0);'
             'for(int i=0;i<16;i++)store(0x1800+4*i,c[1+i]);for(int i=0;i<4;i++)for(int j=0;j<4;j++)store(0x1840+16*i+4*j,i==j?0x3F800000:0);'
             'store(0x1000+1260,n);store(0x1000+1264,0xFFFFFFFF);'
             'for(uint32_t k=0;k<n;k++){const uint32_t*r=c+17+15*k;uint32_t at=0x1000+192+176*k;'
             'store16(at+174,uint16_t(r[0]));store(at+12,r[1]);'
             'for(int i=0;i<3;i++){store(at+16+4*i,r[2+i]);store(at+32+4*i,r[5+i]);store(at+64+4*i,r[5+i]);store(at+96+4*i,r[8+i]);'
             'store(at+80+4*i,r[8+i]^0x80000000);store(at+48+4*i,r[12+i]);store(at+144+4*i,0x3F000000);}'
             'store(at+160,r[11]);}')
    body = ('static const uint32_t C[][%d]={' % (17 + 15 * MAXN) + nl + (',' + nl).join(flat) + nl + '};' + nl +
            'for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);store(0x8286093C,0x3F800000);'
            'printf("I %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x' + chr(92) + 'n",'
            'load(0x82081BF4),load(0x828608E0),load(0x828608E4),load(0x828608E8),load(0x828608EC),load(0x82860930),load(0x82860934),'
            'load(0x82860938),load(0x8286093C),load(0x82860950),load(0x82860954),load(0x82860958),load(0x8286095C),load(0x82812AA0));'
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r5.u32=0x1840;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);'
            'uint32_t m=load(0x1000+1260);printf("R %u %08x",m,load(0x1000+1264));for(uint32_t k=0;k<m&&k<6;k++){uint32_t at=0x1000+192+176*k;'
            'printf(" %u %u %08x %08x %08x %08x %08x %08x %08x",load(at+160),load16(at+174),load(at+12),load(at+48),load(at+52),load(at+56),'
            'load(at+144),load(at+148),load(at+152));}printf("' + chr(92) + 'n");}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'mref', vector=True, scratch_end=0x8000)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    print(out[0])
    out = out[1:]
    assert len(out) == len(cases), (len(out), len(cases))
    lines, kept = [], 0
    for (m, recs), o in zip(cases, out):
        p = o.split()
        assert p[0] == 'R'
        k = int(p[1])
        kept += k
        w = ['M', '%d' % len(recs)] + ['x%08x' % fb(x) for row in m for x in row]
        for r in recs:
            w += ['|', '%d' % r[0], 'x%08x' % fb(r[1])] + ['x%08x' % fb(x) for x in r[2:11]] + ['%d' % r[11]] + ['x%08x' % fb(x) for x in r[12:15]]
        res = ['%d' % k]
        q = p[3:]
        for i in range(k):
            g = q[9 * i:9 * i + 9]
            res += [g[0], g[1]] + ['x' + x for x in g[2:]]
        lines.append(' '.join(w) + ' -> ' + ' '.join(res))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'manifold_refresh_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases,', sum(len(c[1]) for c in cases), 'contacts in,', kept, 'kept')


if __name__ == '__main__':
    main()
