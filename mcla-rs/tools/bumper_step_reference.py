"""A hanging bumper's frame step run offline: F-44446 (the bumper's per-frame step: clamp the states, forces F-13331,
Verlet, the limits in car space, tearing, the rest frame, the moving frame and the drawn pose) on the generated code
unchanged. The free-piece factory F-10733 is stubbed to answer "no piece" (-1), so a bumper with both ends broken
keeps its hanging pose as on a car without a level.

Bumper object at 0x1000 (the original's offsets): +0 car, +80 / +96 rest points, +112 / +128 end A place / previous,
+144 / +160 A in car space / previous, +176 A force, +192 .. +256 the same for B, +272 the rest frame, +336 M_rest,
+400 the car matrix copy, +468 spring 1000, +472 keep 0.9, +480 clock, +484 front, +485 piece, +496 limits
(0.01, 0.05, 0.15), +512 / +516 states, +528 piece, +532 piece id, +536 max state. Car at 0x2000: +8 -> chassis
(0x2100, +224 speed), +272 its matrix (the thread byte r13+72 set). The frame block's +8 (dt) = 1/30, as running.

Writes <out-dir>/bumper_step_ref.txt: one case a line, "in: <words> -> out: <words>" (hex float / int words):
in = M(16) M_rest(16) restA restB posA prevA locA locprevA posB prevB locB locprevB (4 each) clock speed front
stateA stateB maxstate; out = pose(16) posA prevA locA locprevA forceA posB prevB locB locprevB forceB (4 each)
restframe(16) clock stateA stateB. Test `damageparts::tests::bumper_step_matches_the_original`."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import recomp_leaf

recomp_closure.EXTRA += '#define ctr r2\n#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("unexpected indirect call")\n'
recomp_closure.shim = lambda: recomp_leaf.shim(0x8000)
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
# (the recomp's own x86 flags: vmsum4fp through the CPU's dpps, as the generated code is built)
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def mat(r, place=5.0):
    # a turn about y, a tilt about z and a roll about x, at a random place (rows: x, y, z, place)
    a, t, s = r.uniform(-3, 3), r.uniform(-0.4, 0.4), r.uniform(-0.4, 0.4)
    ca, sa, ct, st, cs, ss = math.cos(a), math.sin(a), math.cos(t), math.sin(t), math.cos(s), math.sin(s)
    Y = [[ca, 0, -sa], [0, 1, 0], [sa, 0, ca]]
    Z = [[ct, st, 0], [-st, ct, 0], [0, 0, 1]]
    X = [[1, 0, 0], [0, cs, ss], [0, -ss, cs]]
    mul = lambda p, q: [[sum(p[i][k] * q[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    m = mul(mul(X, Z), Y)
    return [m[0][0], m[0][1], m[0][2], 0, m[1][0], m[1][1], m[1][2], 0, m[2][0], m[2][1], m[2][2], 0,
            r.uniform(-place, place), r.uniform(-1, 1), r.uniform(-place, place), 1]


def xform(m, p):
    return [p[0] * m[0] + p[1] * m[4] + p[2] * m[8] + m[12], p[0] * m[1] + p[1] * m[5] + p[2] * m[9] + m[13],
            p[0] * m[2] + p[1] * m[6] + p[2] * m[10] + m[14], 1.0]


# (never reached with no piece: the piece's attach and the level's removal, stubbed so their locks stay out)
IDLE = ['F-82611', 'F-31667']


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-44446')
    rnd = random.Random(7)
    cases = []
    for k in range(a.cases):
        m, mr = mat(rnd, 50.0), mat(rnd, 2.0)
        front = rnd.random() < 0.5
        ra = xform(mr, [0.5, 0, 0])
        rb = xform(mr, [-0.5, 0, 0])
        def end(rest, big):
            w = xform(m, rest)
            s = 0.6 if big else 0.08
            pos = [w[i] + rnd.uniform(-s, s) for i in range(3)] + [1.0]
            prev = [pos[i] + rnd.uniform(-s / 4, s / 4) for i in range(3)] + [1.0]
            loc = [rest[i] + rnd.uniform(-0.05, 0.05) for i in range(3)] + [1.0]
            locp = [loc[i] + rnd.uniform(-0.01, 0.01) for i in range(3)] + [1.0]
            return pos + prev + loc + locp
        sa, sb = rnd.choice((0, 1, 1, 2, 2)), rnd.choice((0, 1, 1, 2, 2))
        c = m + mr + ra + rb + end(ra, sa == 2) + end(rb, sb == 2) + [rnd.uniform(0, 200), rnd.choice((0.0, rnd.uniform(0, 90)))]
        cases.append((c, [int(front), sa, sb, rnd.choice((2, 2, 2, 1, 0))]))
    rows = ',\n'.join('{%s}' % ','.join(['0x%08x' % fb(float(x)) for x in c] + ['%d' % x for x in ints]) for c, ints in cases)
    n = len(cases[0][0]) + 4
    # (the vector unit's constants a static initialiser fills: the w mask and w = 1, the 1.0 / 0.5 splats, inf)
    setup = ('memset(objects,0,0x7000);'
             'for(int k=0;k<4;k++){store(0x827D85D0+16*k+4*k,0x3f800000);store(0x827D8450+4*k,0x3f800000);store(0x827D8460+4*k,0x3f000000);store(0x828609E0+4*k,0x7f800000);'
             'store(0x82860950+4*k,k<3?0xffffffff:0);store(0x82860930+4*k,k<3?0:0x3f800000);}'
             'store(0x8288DC40,0);store(0x827D7508,0x3d088889);store(0x5800,0x5840);*pointer(0x5840+72)=1;'
             'store(0x1000,0x2000);store(0x2000+8,0x2100);store(0x2100+224,c[73]);'
             'for(int k=0;k<16;k++){store(0x2000+272+4*k,c[k]);store(0x1000+336+4*k,c[16+k]);}'
             'for(int k=0;k<4;k++){store(0x1000+80+4*k,c[32+k]);store(0x1000+96+4*k,c[36+k]);'
             'for(int j=0;j<4;j++){store(0x1000+112+16*j+4*k,c[40+4*j+k]);store(0x1000+192+16*j+4*k,c[56+4*j+k]);}}'
             'store(0x1000+468,0x447a0000);store(0x1000+472,0x3f666666);store(0x1000+480,c[72]);'
             '*pointer(0x1000+484)=uint8_t(c[74]);*pointer(0x1000+485)=0;'
             'store(0x1000+496,0x3c23d70a);store(0x1000+500,0x3d4ccccd);store(0x1000+504,0x3e19999a);'
             'store(0x1000+512,c[75]);store(0x1000+516,c[76]);store(0x1000+528,0x2400);store(0x1000+532,0xffffffff);store(0x1000+536,c[77]);')
    nl = chr(10)
    outs = ('for(int o=16;o<80;o+=4)printf("%08x ",load(0x1000+o));'
            'for(int o=112;o<272;o+=4)printf("%08x ",load(0x1000+o));'
            'for(int o=272;o<336;o+=4)printf("%08x ",load(0x1000+o));'
            'printf("%08x %08x %08x' + chr(92) + 'n",load(0x1000+480),load(0x1000+512),load(0x1000+516));')
    body = ('static const uint32_t C[][%d]={' % n + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);' + outs + '}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'bumperstep', stubs={'sub_' + addr(a.tool, 'F-10733'): 'ctx.r3.u64=0xffffffffu;', **{'sub_' + addr(a.tool, f): '' for f in IDLE}},
                               vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%08x' % fb(float(x)) for x in c) + ' ' + ' '.join(str(x) for x in ints) + ' -> ' + o for (c, ints), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'bumper_step_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
