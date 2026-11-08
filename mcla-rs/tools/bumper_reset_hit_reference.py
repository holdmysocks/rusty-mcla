"""The bumper's reset F-40944 and its queued-hit rule F-57572 run offline on the generated code unchanged.

Reset: the bumper's rest-frame setup F-27388 runs as it is; the level's removal of a free piece F-27119 is stubbed to
print that it was called. Hit: runs whole (it calls nothing).

Bumper object at 0x1000 (the original's offsets): +0 car, +80 / +96 rest points, +112 / +128 end A place / previous,
+144 / +160 A in car space / previous, +176 A force, +192 .. +256 the same for B, +476 the heavy-damage ratio, +480
clock, +485 piece, +486 the never-set byte, +512 / +516 states, +532 piece id. Car at 0x2000 (the thread byte r13+72
set): +208 the matrix the reset reads, +272 the one the hit (and the rest frame) reads, +8 -> chassis (+128 -> its
body: +272 velocity), +20 -> damage (+36 damage, +8 -> +24 the maximum).

Writes <out-dir>/bumper_reset_ref.txt ("in: <words> -> out: <words>": in = matrix(16) restA restB (4 each) clock
stateA stateB piece-id; out = +112 .. +272 (40 words) clock stateA stateB piece byte piece id removed) and
<out-dir>/bumper_hit_ref.txt (in = matrix(16) restA restB velocity point (4 each) amount damage maximum ratio dt
kind stateA stateB; out = stateA stateB +486).
Tests `damageparts::tests::bumper_reset_matches_the_original`, `bumper_hit_matches_the_original`."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import recomp_leaf

recomp_closure.EXTRA += 'static int removed;\n#define ctr r2\n#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("unexpected indirect call")\n'
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
            p[0] * m[2] + p[1] * m[6] + p[2] * m[10] + m[14]]


# (the vector unit's constants a static initialiser fills: the identity, the w mask and w = 1; the thread byte)
COMMON = ('memset(objects,0,0x7000);'
          'for(int k=0;k<4;k++){store(0x827D85D0+16*k+4*k,0x3f800000);store(0x82860950+4*k,k<3?0xffffffff:0);'
          'store(0x82860930+4*k,k<3?0:0x3f800000);store(0x827D8450+4*k,0x3f800000);store(0x827D8460+4*k,0x3f000000);}'
          'store(0x5800,0x5840);*pointer(0x5840+72)=1;store(0x1000,0x2000);')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    reset, hit = 'sub_' + addr(a.tool, 'F-40944'), 'sub_' + addr(a.tool, 'F-57572')
    rnd = random.Random(40944)
    nl = chr(10)

    # ---- the reset
    rc = []
    for k in range(a.cases):
        m, mr = mat(rnd, 50.0), mat(rnd, 2.0)
        ra, rb = xform(mr, [0.5, 0, 0]) + [rnd.choice((1.0, 0.0, rnd.uniform(-2, 2)))], xform(mr, [-0.5, 0, 0]) + [rnd.choice((1.0, 0.0))]
        rc.append((m + ra + rb + [rnd.uniform(0, 200)], [rnd.randint(0, 2), rnd.randint(0, 2), rnd.choice((-1, -1, 7, 1234))]))
    rrows = ',\n'.join('{%s}' % ','.join(['0x%08x' % fb(float(x)) for x in c] + ['%du' % (x & 0xffffffff) for x in i]) for c, i in rc)
    rsetup = (COMMON + 'for(int k=0;k<16;k++){store(0x2000+208+4*k,c[k]);store(0x2000+272+4*k,c[k]);}'
              'for(int k=0;k<4;k++){store(0x1000+80+4*k,c[16+k]);store(0x1000+96+4*k,c[20+k]);}'
              'for(int o=112;o<272;o+=4)store(0x1000+o,0x40490fdb);'
              'store(0x1000+480,c[24]);store(0x1000+512,c[25]);store(0x1000+516,c[26]);*pointer(0x1000+485)=1;store(0x1000+532,c[27]);removed=0;')
    rout = ('for(int o=112;o<272;o+=4)printf("%08x ",load(0x1000+o));'
            'printf("%08x %08x %08x %d %08x %d' + chr(92) + 'n",load(0x1000+480),load(0x1000+512),load(0x1000+516),'
            '*pointer(0x1000+485),load(0x1000+532),removed);')

    # ---- the hit
    hc = []
    for k in range(a.cases):
        m, mr = mat(rnd, 50.0), mat(rnd, 2.0)
        ra, rb = xform(mr, [0.5, 0, 0]), xform(mr, [-0.5, 0, 0])
        end = xform(m, rnd.choice((ra, rb)))
        r = rnd.choice((rnd.uniform(0, 1.2), rnd.uniform(0.79, 0.81), 0.8, 0.0))
        d = [rnd.gauss(0, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in d)) or 1.0
        pt = [end[i] + d[i] / n * r for i in range(3)]
        vel = [rnd.uniform(-40, 40) for _ in range(3)]
        amount = rnd.choice((rnd.uniform(0, 1), 0.15, 0.15 + 2 ** -24, math.nan, 0.0, 5.0))
        mx = rnd.choice((1000.0, rnd.uniform(1, 5000)))
        dmg = rnd.choice((0.0, mx * rnd.uniform(0, 1), mx * 0.66, mx))
        hc.append((m + ra + [1.0] + rb + [1.0] + vel + [0.0] + pt + [1.0] + [amount, dmg, mx, rnd.choice((0.66, 0.5)), rnd.choice((1 / 30, 1 / 60))],
                   [rnd.choice((0, 0, 0, 1)), rnd.randint(0, 2), rnd.randint(0, 2)]))
    hrows = ',\n'.join('{%s}' % ','.join(['0x%08x' % fb(float(x)) for x in c] + ['%d' % x for x in i]) for c, i in hc)
    hsetup = (COMMON + 'for(int k=0;k<16;k++)store(0x2000+272+4*k,c[k]);'
              'for(int k=0;k<4;k++){store(0x1000+80+4*k,c[16+k]);store(0x1000+96+4*k,c[20+k]);store(0x2200+272+4*k,c[24+k]);store(0x2600+4*k,c[28+k]);}'
              'store(0x2000+8,0x2100);store(0x2100+128,0x2200);store(0x2000+20,0x2300);store(0x2300+36,c[33]);store(0x2300+8,0x2400);'
              'store(0x2400+24,c[34]);store(0x1000+476,c[35]);store(0x827D7508,c[36]);*pointer(0x1000+486)=0;'
              'store(0x1000+512,c[38]);store(0x1000+516,c[39]);')
    hout = 'printf("%d %d %d' + chr(92) + 'n",load(0x1000+512),load(0x1000+516),*pointer(0x1000+486));'

    body = (
            'static const uint32_t R[][%d]={' % (len(rc[0][0]) + 3) + nl + rrows + nl + '};' + nl +
            'static const uint32_t H[][%d]={' % (len(hc[0][0]) + 3) + nl + hrows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
            'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&cc:R){const uint32_t*c=cc;' + rsetup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + reset + '(ctx,nullptr);printf("R ");' + rout + '}' + nl +
            'for(auto&cc:H){const uint32_t*c=cc;' + hsetup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x2600;ctx.r5.u64=c[37];float am;memcpy(&am,&c[32],4);ctx.f1.f64=am;'
            'ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + hit + '(ctx,nullptr);printf("H ");' + hout + '}}' + nl)
    # (the removal sees the piece id it is handed: printed through `removed`)
    stubs = {'sub_' + addr(a.tool, 'F-27119'): 'removed=1;'}
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [reset, hit], body, a.out_dir, 'bumperresethit', stubs=stubs, vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    ro, ho = [x[2:] for x in out if x.startswith('R ')], [x[2:] for x in out if x.startswith('H ')]
    assert len(ro) == len(rc) and len(ho) == len(hc), (len(ro), len(ho))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    for name, cs, os_ in (('bumper_reset_ref.txt', rc, ro), ('bumper_hit_ref.txt', hc, ho)):
        lines = [' '.join('%08x' % fb(float(x)) for x in c) + ' ' + ' '.join(str(x) for x in i) + ' -> ' + o for (c, i), o in zip(cs, os_)]
        (a.out_dir / name).write_text(nl.join(lines) + nl)
    print(len(ro), len(ho), 'cases')


if __name__ == '__main__':
    main()
