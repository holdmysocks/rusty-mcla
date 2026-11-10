"""The hood's / trunk lid's frame step run offline: F-16752 (the per-frame wrapper: the hinge to world, gravity and the
Verlet move of a loose flap, else the angle 0) with F-50514 (to car space, the wind lift F-66314, the stops, back into
the car's YZ plane, the angle) on the generated code unchanged. Each flap is built as the car-anim binding F-53023
builds it: P0 = (0, 2, 0), R = Pc = (0, 2, 1), Po = P0 + rotate(R - P0) by the image's angle (0x8202DC38 hood,
0x82046B90 trunk, the rotation F-43547), the set-up F-71271, the reset F-97771 at the car's matrix; then a random
state (loose, particle, last particle, angle, last angle). The stop sounds F-28953 (closed) / F-47714 (open) are
stubbed to log which and their rate (f1).

Flap at 0x1000 (the original's offsets, DAMAGE_PARTS_RULE.md 1.1). Car at 0x2000: +8 -> chassis (0x2400, +224 speed),
+16 the sound object, +144 / +208 its matrix (the thread byte r13+72 set: +208). The frame block's +8 (dt) = 1/30, +12
(1/dt) = 1 / dt = 29.999998 (its writer F-75502 divides 1.0 by dt), as running.

Writes <out-dir>/flap_step_ref.txt: one case a line, "in -> out" (hex float / int words): in = M(16) p(4) q(4) angle
prev speed (floats) trunk loose (ints); out = flap +0 .. +188 (48 words), the event (0 none, 1 closed, 2 open) and its
rate, then the reset's flap +112 .. +188 (20 words, before the random state). Tests
`damageparts::tests::flap_step_matches_the_original`, `flap_reset_matches_the_original`."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs


fb, addr, mat, xform = bs.fb, bs.addr, bs.mat, bs.xform


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    f = {k: 'sub_' + addr(a.tool, k) for k in ['F-16752', 'F-43547', 'F-71271', 'F-97771', 'F-28953', 'F-47714']}
    rnd = random.Random(11)
    cases = []
    for k in range(a.cases):
        m = mat(rnd, 50.0)
        trunk = rnd.random() < 0.5
        loose = rnd.random() < 0.8
        r = xform(m, [0.0, 2.0, 1.0])
        # the particle: round the rest point (a swing, sideways too), the last place a frame's move away
        s = rnd.choice((0.02, 0.1, 0.3))
        p = [r[i] + rnd.uniform(-s, s) for i in range(3)] + [1.0]
        v = rnd.choice((0.0, 0.01, 0.1))
        q = [p[i] + rnd.uniform(-v, v) for i in range(3)] + [1.0]
        amax = 0.15 if trunk else 0.2
        ang = rnd.choice((0.0, -rnd.uniform(0, amax), -rnd.uniform(0, 0.05)))
        prev = rnd.choice((ang, 0.0, -rnd.uniform(0, amax)))
        speed = rnd.choice((0.0, rnd.uniform(0, 3), rnd.uniform(0, 80)))
        cases.append((m + p + q + [ang, prev, speed], [int(trunk), int(loose)]))
    rows = ',\n'.join('{%s}' % ','.join(['0x%08x' % fb(float(x)) for x in c] + ['%d' % x for x in ints]) for c, ints in cases)
    n = len(cases[0][0]) + 2
    nl = chr(10)
    setup = ('memset(objects,0,0x7000);'
             'for(int k=0;k<4;k++){store(0x827D85D0+16*k+4*k,0x3f800000);store(0x827D8450+4*k,0x3f800000);store(0x827D8460+4*k,0x3f000000);store(0x828609E0+4*k,0x7f800000);'
             'store(0x82860950+4*k,k<3?0xffffffff:0);store(0x82860930+4*k,k<3?0:0x3f800000);}'
             'store(0x8288DC40,0);store(0x827D7508,0x3d088889);store(0x827D750C,0x41efffff);store(0x5800,0x5840);*pointer(0x5840+72)=1;'
             'store(0x2000+8,0x2400);store(0x2000+16,0x2600);store(0x2400+224,c[26]);'
             'for(int k=0;k<16;k++){store(0x2000+144+4*k,c[k]);store(0x2000+208+4*k,c[k]);}'
             # the binding's points (F-53023): P0 at 0x2800, R 0x2810, Po 0x2820, Pc 0x2830
             'const uint32_t two=0x40000000,one=0x3f800000;'
             'store(0x2804,two);store(0x2814,two);store(0x2818,one);store(0x2834,two);store(0x2838,one);'
             'store(0x2824,0);store(0x2828,one);'
             '{Context t{};t.r3.u32=0x2820;t.r13.u32=0x5800;t.r1.u32=0x7F00;uint32_t w=load(c[27]?0x82046B90:0x8202DC38);float x;memcpy(&x,&w,4);t.f1.f64=x;'
             + f['F-43547'] + '(t,nullptr);}'
             '{uint32_t w=load(0x2824);float y;memcpy(&y,&w,4);y+=2.0f;memcpy(&w,&y,4);store(0x2824,w);}'
             '{Context t{};t.r3.u32=0x1000;t.r4.u32=0x2800;t.r5.u32=0x2810;t.r6.u32=0x2820;t.r7.u32=0x2830;t.r13.u32=0x5800;t.r1.u32=0x7F00;'
             + f['F-71271'] + '(t,nullptr);}'
             '{Context t{};t.r3.u32=0x1000;t.r4.u32=0x2000+208;t.r13.u32=0x5800;t.r1.u32=0x7F00;' + f['F-97771'] + '(t,nullptr);}'
             'for(int o=112;o<192;o+=4)store(0x6100+o-112,load(0x1000+o));'
             'for(int k=0;k<4;k++){store(0x1000+112+4*k,c[16+k]);store(0x1000+128+4*k,c[20+k]);}'
             'store(0x1000+180,c[24]);store(0x1000+184,c[25]);*pointer(0x1000+88)=uint8_t(c[28]);'
             'store(0x6000,0);store(0x6004,0);')
    outs = ('for(int o=0;o<192;o+=4)printf("%08x ",load(0x1000+o));'
            'printf("%08x %08x",load(0x6000),load(0x6004));'
            'for(int o=0;o<80;o+=4)printf(" %08x",load(0x6100+o));printf("' + chr(92) + 'n");')
    log = '{float t=float(ctx.f1.f64);uint32_t u;memcpy(&u,&t,4);store(0x6000,%d);store(0x6004,u);}'
    stubs = {f['F-28953']: log % 1, f['F-47714']: log % 2}
    body = ('static const uint32_t C[][%d]={' % n + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x2000;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + f['F-16752'] + '(ctx,nullptr);' + outs + '}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [f['F-16752'], f['F-43547'], f['F-71271'], f['F-97771']], main_src, a.out_dir, 'flapstep',
                               stubs=stubs, vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%08x' % fb(float(x)) for x in c) + ' ' + ' '.join(str(x) for x in ints) + ' -> ' + o for (c, ints), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'flap_step_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
