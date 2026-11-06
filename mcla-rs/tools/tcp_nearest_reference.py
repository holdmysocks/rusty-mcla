"""The arc-line-arc rail's nearest point F-16977 (slot 14 of mcRailAmbientTCP, through the thunk F-36154) run offline:
each case builds a rail with F-85722 (the build of tcp_rail_reference.py: vector mode, the SDK's flush mode, the
normalisations' +inf guard filled in) and asks F-16977 for 8 query points - near the start P, the tangent points A and
B, the end T, the centres c1 / c2 (and exactly on them: the zero flat way), on the arcs' circles, far away, with
heights - the generated code whole with F-88594 (the 3D angle and the guest's acos), F-17399 and F-56929 in its closure.
Guest memory: the rail record 0x1000, inputs 0x1100 .., the query 0x1300, outputs 0x1200 (point), 0x1310 (phase),
0x1314 (squared distance).
Writes <out-dir>/tcp_nearest_ref.txt, one case a line: P H0 T H1 (xyz each), R, s0, s1, then per query its xyz;
'->' the build's valid flag, then per query the point xyz, the phase and the squared distance (hex bits)."""
import argparse
import math
import random
from pathlib import Path
import tcp_rail_reference as trr
from tcp_rail_reference import addr, fb, f32
from recomp_closure import index, build, run

NQ = 8


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2500)
    a = ap.parse_args()
    bld, near = addr(a.tool, 'F-85722'), addr(a.tool, 'F-16977')
    rnd = random.Random(16977)

    def heading():
        u = rnd.random()
        y = rnd.uniform(-math.pi, math.pi)
        h = [math.sin(y), 0.0, -math.cos(y)]
        if u < 0.1:
            h[1] = rnd.uniform(-0.3, 0.3)
        elif u < 0.2:
            k = rnd.choice((0.5, 2.0, rnd.uniform(0.01, 30)))
            h = [x * k for x in h]
        return h

    def perp(h, s):
        l = math.hypot(h[0], h[2])
        n = [-h[2] / l, 0.0, h[0] / l]
        return n if s else [-x for x in n]

    cases = []
    for _ in range(a.cases):
        R = rnd.choice((0.1, 0.4, 0.4, 2.0, 5.0, rnd.uniform(0.05, 30)))
        P = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        H0, H1 = heading(), heading()
        s0, s1 = rnd.getrandbits(1), rnd.getrandbits(1)
        n0 = perp(H0, s0)
        c1 = [P[i] + R * n0[i] for i in range(3)]
        dist = rnd.choice((rnd.uniform(0, 3), rnd.uniform(0, 30), rnd.uniform(0, 150), 2 * R))
        ang = rnd.uniform(-math.pi, math.pi)
        dy = rnd.choice((0.0, 0.0, rnd.uniform(-6, 6)))
        T = [P[0] + dist * math.cos(ang), P[1] + dy, P[2] + dist * math.sin(ang)]
        if rnd.random() < 0.05:
            T = [c1[0] + R * 0.5, P[1], c1[2]]  # inside the start circle (invalid)
        n1 = perp(H1, s1)
        c2 = [T[i] + R * n1[i] for i in range(3)]
        qs = []
        for k in range(NQ):
            kind = rnd.random()
            base = rnd.choice((P, T, c1, c2, [(P[i] + T[i]) / 2 for i in range(3)]))
            if kind < 0.08:
                q = list(base)  # exactly on a point (a centre: the zero way)
            elif kind < 0.35:
                c = rnd.choice((c1, c2))
                t = rnd.uniform(-math.pi, math.pi)
                r = R * rnd.choice((1.0, rnd.uniform(0.5, 1.5), rnd.uniform(0, 3)))
                q = [c[0] + r * math.cos(t), c[1] + rnd.choice((0.0, rnd.uniform(-2, 2))), c[2] + r * math.sin(t)]
            elif kind < 0.8:
                q = [base[i] + rnd.uniform(-1, 1) * rnd.choice((0.3, 3.0, 20.0)) for i in range(3)]
            else:
                q = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
            qs += q
        cases.append([fb(f32(x)) for x in P + H0 + T + H1 + [R]] + [s0, s1] + [fb(f32(x)) for x in qs])
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);'
            'auto bf=[](uint32_t u){float f;memcpy(&f,&u,4);return f;};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));'
            'for(int i=0;i<4;i++)for(int k=0;k<3;k++)store(0x1100+16*i+4*k,c[3*i+k]);'
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r5.u32=0x1110;ctx.r6.u32=c[13];'
            'ctx.r7.u32=0x1120;ctx.r8.u32=0x1130;ctx.r9.u32=c[14];ctx.f1.f64=bf(c[12]);'
            'sub_%s(ctx,nullptr);printf("%%d",int(ctx.r3.u32));' % bld +
            'for(int q=0;q<%d;q++){for(int k=0;k<3;k++)store(0x1300+4*k,c[15+3*q+k]);store(0x130C,0);' % NQ +
            'Context x{};x.r1.u32=0x2F00;x.r3.u32=0x1200;x.r4.u32=0x1000;x.r5.u32=0x1300;x.r6.u32=0x1310;x.r7.u32=0x1314;'
            'sub_%s(x,nullptr);for(int k=0;k<3;k++)printf(" %%08x",load(0x1200+4*k));printf(" %%08x %%08x",load(0x1310),load(0x1314));}' % near +
            'printf("' + NL + '");}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + bld, 'sub_' + near], main_src, a.out_dir, 'tcp_nearest', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join(('%d' % x) if i in (13, 14) else '%08x' % x for i, x in enumerate(c)) + ' -> ' + o for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'tcp_nearest_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] == '1' for o in out), 'valid,', len(lines) * NQ, 'queries')


if __name__ == '__main__':
    main()
