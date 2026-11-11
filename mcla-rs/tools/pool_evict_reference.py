"""The fragment cache pool's choice of a victim run offline: F-18636 (the pool's allocate) on the generated code
unchanged, with the pool full and the viewer sphere set (manager +776 = 1, as in play), and F-94385 (a bound sphere
carried by a matrix). Reference: docs/reference/vehicle/prop_reset_2026-11-10.md "The pool".

Pool (r3) at 0x1000: +0 cap, +4 used, +8 frame, +12 the list (+12 back, +16 front, +20 count). The manager at 0x1400
(global 0x828D4CA4 -> it): +752 -> the pool, +736 the viewer sphere, +776 = 1. Entry k at 0x2000 + 0x100 k: +12 stamp,
+232 -> its instance, +244 next, +248 previous. Instance k at 0x3800 + 0x100 k: +0 -> the vtable 0x5800 (slot 44, +176
-> 0x90000000: "the type keeps its entry", answered from the instance's byte +200), +96 the bound sphere. The teardown
F-74663 is stubbed (it logs its entry); the stamp F-95559 runs as generated. Start-up constants the image holds as zero:
0x82860950 = xyz all ones (w 0), 0x82860930 = (0, 0, 0, 1). F-94385: sphere at 0x6100, matrix at 0x6200.

Writes <out-dir>/pool_evict_ref.txt: "V frame cx cy cz R n | stamp keeps x y z r ... -> victim" (hex words, victim the
entry's index in the list from the front or -1) and "S m00 .. m32 cx cy cz r -> x y z r". Test
`physics::loose::tests::pool_victim_matches_the_original`."""
import argparse
import math
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)

recomp_closure.EXTRA = recomp_closure.EXTRA.replace('#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("unexpected indirect call")\n', '')
recomp_closure.EXTRA += 'static void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
fb, addr = bs.fb, bs.addr
MAXN = 24


def case(rnd):
    frame = rnd.choice([rnd.randrange(3, 100000), 0, 1, 2, 0xFFFFFFFF])
    c = [rnd.uniform(-3000, 3000), rnd.uniform(-50, 50), rnd.uniform(-3000, 3000)]
    big = rnd.choice([300.0, 300.0, 300.0, 0.0, rnd.uniform(1, 400)])
    n = rnd.randrange(1, MAXN + 1)
    rows = []
    reach = rnd.choice([50.0, 300.0, 330.0, 700.0])
    for _ in range(n):
        stamp = rnd.choice([frame, (frame - 1) & 0xFFFFFFFF, (frame - 2) & 0xFFFFFFFF, rnd.randrange(0, max(frame, 1)), rnd.randrange(0, 1 << 32)])
        keeps = 1 if rnd.random() < 0.15 else 0
        if rows and rnd.random() < 0.1:
            s = list(rows[rnd.randrange(len(rows))][2:])
        else:
            d = [rnd.uniform(-1, 1) for _ in range(3)]
            l = math.sqrt(sum(x * x for x in d)) or 1.0
            dist = rnd.uniform(0, reach)
            s = [c[i] + d[i] / l * dist for i in range(3)] + [rnd.choice([rnd.uniform(0.05, 10.0), 0.0])]
        if rnd.random() < 0.01:
            s[rnd.randrange(4)] = float('nan')
        rows.append([stamp, keeps] + s)
    return frame, c + [big], rows


def sphere_case(rnd):
    a, t, s = rnd.uniform(-3, 3), rnd.uniform(-0.4, 0.4), rnd.uniform(-0.4, 0.4)
    ca, sa, ct, st, cs, ss = math.cos(a), math.sin(a), math.cos(t), math.sin(t), math.cos(s), math.sin(s)
    rx = [ca * ct, st, -sa * ct]
    ry = [-ca * st * cs + sa * ss, ct * cs, sa * st * cs + ca * ss]
    rz = [ca * st * ss + sa * cs, -ct * ss, -sa * st * ss + ca * cs]
    k = rnd.choice([1.0, 1.0, rnd.uniform(0.3, 3.0)])
    rows = [[x * k * rnd.choice([1.0, 1.0, rnd.uniform(0.8, 1.2)]) for x in r] for r in (rx, ry, rz)]
    if rnd.random() < 0.02:
        rows = [[0.0] * 3] * 3
    place = [rnd.uniform(-3000, 3000), rnd.uniform(-50, 50), rnd.uniform(-3000, 3000)]
    sph = [rnd.uniform(-5, 5) for _ in range(3)] + [rnd.uniform(0, 20)]
    return rows + [place], sph


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn, sph = 'sub_' + addr(a.tool, 'F-18636'), 'sub_' + addr(a.tool, 'F-94385')
    teardown = 'sub_' + addr(a.tool, 'F-74663')
    rnd = random.Random(48)
    cases = [case(rnd) for _ in range(a.cases)]
    scases = [sphere_case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    flat = []
    for frame, vw, rows in cases:
        w = ['0x%08x' % frame] + ['0x%08x' % fb(x) for x in vw] + ['%d' % len(rows)]
        for r in rows + [[0, 0, 0.0, 0.0, 0.0, 0.0]] * (MAXN - len(rows)):
            w += ['0x%08x' % r[0], '%d' % r[1]] + ['0x%08x' % fb(x) for x in r[2:]]
        flat.append('{' + ','.join(w) + '}')
    sflat = ['{' + ','.join('0x%08x' % fb(x) for x in sum(m, []) + s) + '}' for m, s in scases]
    consts = ('for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);store(0x8286093C,0x3F800000);'
              'for(int k=0;k<4;k++){store(0x828608E0+4*k,0);store(0x828609E0+4*k,0x7F800000);store(0x828608C0+4*k,0);store(0x827D8450+4*k,0x3F800000);store(0x827D8460+4*k,0x3F000000);}')
    setup = ('for(uint32_t q=0x1000;q<0x7000;q+=4)store(q,0);store(0x828D4CA4,0x1400);store(0x1400+752,0x1000);*pointer(0x1400+776)=1;'
             'for(int i=0;i<4;i++)store(0x1400+736+4*i,c[1+i]);store(0x5800+176,0x90000000);'
             'uint32_t n=c[5];store(0x1000,n);store(0x1004,n);store(0x1008,c[0]);store(0x1010,0x2000);store(0x100C,0x2000+0x100*(n-1));store(0x1014,n);'
             'for(uint32_t k=0;k<n;k++){const uint32_t*r=c+6+6*k;uint32_t e=0x2000+0x100*k,in=0x3800+0x100*k;'
             'store(e+12,r[0]);store(e+232,in);store(e+244,k+1<n?e+0x100:0);store(e+248,k?e-0x100:0);'
             'store(in,0x5800);*pointer(in+200)=uint8_t(r[1]);for(int i=0;i<4;i++)store(in+96+4*i,r[2+i]);}')
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a!=0x90000000)throw std::runtime_error("indirect");'
            'ctx.r3.u64=*pointer(ctx.r3.u32+200);}')
    body = ('static const uint32_t C[][%d]={' % (6 + 6 * MAXN) + nl + (',' + nl).join(flat) + nl + '};' + nl +
            'static const uint32_t S[][20]={' + nl + (',' + nl).join(sflat) + nl + '};' + nl + consts +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup + 'store(0x6000,0);'
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);'
            'uint32_t v=ctx.r3.u32;printf("V %d %08x' + chr(92) + 'n",v?int((v-0x2000)/0x100):-1,load(0x6000));}'
            'for(auto&s:S){for(int i=0;i<16;i++)store(0x6200+4*i,0);for(int r=0;r<4;r++)for(int i=0;i<3;i++)store(0x6200+16*r+4*i,s[3*r+i]);'
            'for(int i=0;i<4;i++)store(0x6100+4*i,s[12+i]);Context ctx{};ctx.r3.u32=0x6100;ctx.r4.u32=0x6200;ctx.r1.u32=0x7F00;' + sph + '(ctx,nullptr);'
            'printf("S %08x %08x %08x %08x' + chr(92) + 'n",load(0x6100),load(0x6104),load(0x6108),load(0x610C));}')
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    stubs = {teardown: 'store(0x6000,ctx.r3.u32);'}
    exe = recomp_closure.build(idx, [fn, sph], main_src, a.out_dir, 'poolevict', stubs=stubs, vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    vs = [x for x in out if x.startswith('V ')]
    ss = [x for x in out if x.startswith('S ')]
    assert len(vs) == len(cases) and len(ss) == len(scases), (len(vs), len(ss))
    lines = []
    for (frame, vw, rows), o in zip(cases, vs):
        v, t = o.split()[1:]
        if int(v) >= 0:
            assert int(t, 16) == 0x2000 + 0x100 * int(v), o
        w = ['V', '%08x' % frame] + ['%08x' % fb(x) for x in vw] + ['%d' % len(rows)]
        for r in rows:
            w += ['|', '%08x' % r[0], '%d' % r[1]] + ['%08x' % fb(x) for x in r[2:]]
        lines.append(' '.join(w) + ' -> ' + v)
    for (m, s), o in zip(scases, ss):
        lines.append(' '.join(['S'] + ['%08x' % fb(x) for x in sum(m, []) + s]) + ' -> ' + ' '.join(o.split()[1:]))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'pool_evict_ref.txt').write_text(nl.join(lines) + nl)
    took = sum(1 for x in vs if x.split()[1] != '-1')
    print(len(cases), 'pool cases,', took, 'with a victim;', len(scases), 'sphere cases')


if __name__ == '__main__':
    main()
