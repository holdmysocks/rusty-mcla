"""Run the original's particle collision step F-99991 offline (the generated code as it is, with the swept test F-22746
and the ball-polygon test F-17804 whole) and record what it does to the particle, for the port's test
(crates/game/src/testdata/particle_collide_reference.txt).

Stubbed: the keyframe evaluation F-42828 (it writes the case's (lo, hi) pair of VelDmp for keys rule+980 and of ImpVar
for rule+1032: the evaluation itself is checked by its own row). The particle's two random bytes +137 / +138 index the
random float table, filled with the case's weights at those two places. The game generator's state (the global's +0
word and +4 carry) is the case's. The start-up vector constants are set as the running game has them (0.5 and 1.0 x4,
+inf x4, 0 x4, the xyz mask, (0, 0, 0, 1)).

The rule: +280 StopVel, +284 bit 0 kill on, +307 PercentKill, +1029 / +1081 the two weights' invert flags. The instance:
+512 the polygon list (+0 entries, +4 u16 count; 96-byte entries: four corners +0 .. +48, the normal +64, the count of
corners +80). The particle: +132 flags, +192 place, +224 velocity, +256 life.

  CASE <radius> <dt> <stop> <kill on> <pct> <dmp lo hi w inv> <imp lo hi w inv> <gen x c (decimal, g-prefixed)> <pos xyz> <vel xyz> <life> <npoly>
  POLY <v0 v1 v2 v3 xyz> <n xyz> <corners>
  OUT <ret> <pos xyz> <vel xyz> <life> <flags> <gen x c (decimal, g-prefixed)>

(The generator's words are written in decimal: as hex some read like addresses of the original.)

python particle_collide_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10) + 'static uint32_t DL, DH, IL, IH;' + chr(10) + '#define ppc_trap(c, b, x) throw std::runtime_error("trap")' + chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


# globals: the generator's pointer, the random float table, the start-up vectors (0.5, 1.0, +inf, 0, xyz mask, (0,0,0,1))
GEN, TABLE = 0x828D4D18, 0x828D51F0
VECS = {0x827D8460: [0x3F000000] * 4, 0x827D8450: [0x3F800000] * 4, 0x828609E0: [0x7F800000] * 4, 0x828608C0: [0] * 4,
        0x82860950: [0xFFFFFFFF] * 3 + [0], 0x82860930: [0, 0, 0, 0x3F800000]}


def unit(v):
    n = math.sqrt(sum(x * x for x in v))
    return [x / n for x in v]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def cases(rnd):
    out = []
    for k in range(3000):
        n = unit([rnd.uniform(-0.3, 0.3), 1.0, rnd.uniform(-0.3, 0.3)]) if k % 3 else unit([rnd.uniform(-1, 1) for _ in range(3)])
        t1 = unit(cross(n, [1.0, 0.0, 0.0] if abs(n[0]) < 0.9 else [0.0, 0.0, 1.0]))
        t2 = cross(n, t1)
        c = [rnd.uniform(-5, 5) for _ in range(3)]
        polys = []
        for p in range(rnd.choice((1, 1, 2, 3))):
            sx, sy = rnd.uniform(0.3, 3), rnd.uniform(0.3, 3)
            off = [rnd.uniform(-0.5, 0.5) * p * x for x in t1]
            cc = [c[i] + off[i] + n[i] * rnd.choice((0.0, 0.0, rnd.uniform(-0.2, 0.2))) for i in range(3)]
            corner = lambda a, b: [cc[i] + a * sx * t1[i] + b * sy * t2[i] for i in range(3)]
            vs = [corner(-1, -1), corner(1, -1), corner(1, 1), corner(-1, 1)]
            if rnd.random() < 0.5:  # the other winding
                vs = vs[::-1]
            nn = n if rnd.random() < 0.85 else [-x for x in n]
            corners = rnd.choice((4, 4, 3))
            polys.append((vs, nn, corners))
            if p == 0:
                s1, s2 = sx, sy
        radius = rnd.choice((0.05, 0.1, 0.3, rnd.uniform(0.01, 1)))
        dt = rnd.choice((1 / 30, 1 / 60, rnd.uniform(0.001, 0.1)))
        # the particle: near the first polygon's plane, moving towards it (or not)
        a, b = rnd.uniform(-1.2, 1.2), rnd.uniform(-1.2, 1.2)
        h = rnd.choice((rnd.uniform(-radius, radius), rnd.uniform(-2 * radius, 2 * radius), rnd.uniform(-0.5, 0.5), 0.0))
        pos = [c[i] + a * s1 * t1[i] + b * s2 * t2[i] + n[i] * h for i in range(3)]
        sp = rnd.choice((rnd.uniform(0, 2), rnd.uniform(0, 30), 0.0, radius * 0.7 / dt * rnd.uniform(0.9, 1.1)))
        g = rnd.uniform(0.3, 4) if rnd.random() < 0.8 else rnd.uniform(-1, 1)
        dirv = unit([rnd.uniform(-1, 1) - n[i] * g for i in range(3)])
        vel = [x * sp for x in dirv]
        if rnd.random() < 0.4:  # crossing the plane this step (the swept test) or ending within the radius
            h = rnd.uniform(-0.9, 0.9) * max(sp * dt, radius)
            pos = [c[i] + a * s1 * t1[i] + b * s2 * t2[i] + n[i] * h for i in range(3)]
        if rnd.random() < 0.05:
            vel = [0.0, 0.0, 0.0]
        stop = rnd.choice((0.0, 0.5, 2.0, rnd.uniform(0, 5)))
        kill = rnd.choice((0, 1))
        pct = rnd.choice((0, 50, 100, 255, rnd.randrange(0, 256)))
        dmp = [rnd.choice((0.0, 50.0, 100.0, rnd.uniform(0, 100))) for _ in range(2)] + [rnd.random(), rnd.choice((0, 1))]
        imp = [rnd.choice((0.0, 0.0, 0.2, rnd.uniform(0, 1))) for _ in range(2)] + [rnd.random(), rnd.choice((0, 1))]
        gen = [rnd.getrandbits(32), rnd.getrandbits(32)]
        life = rnd.uniform(0, 1)
        w = [fbits(f32(radius)), fbits(f32(dt)), fbits(f32(stop)), kill, pct, fbits(f32(dmp[0])), fbits(f32(dmp[1])), fbits(f32(dmp[2])), dmp[3],
             fbits(f32(imp[0])), fbits(f32(imp[1])), fbits(f32(imp[2])), imp[3], gen[0], gen[1]] + [fbits(f32(x)) for x in pos + vel] + [fbits(f32(life)), len(polys)]
        for vs, nn, corners in polys:
            w += [fbits(f32(x)) for v in vs for x in v] + [fbits(f32(x)) for x in nn] + [corners]
        out.append(w)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, ev = addr(a.tool, 'F-99991'), addr(a.tool, 'F-42828')
    cs = cases(random.Random(99991))
    nl, NL = chr(10), chr(92) + 'n'
    # F-42828 (r4 out, r5 the keys): VelDmp keys at rule+980, ImpVar at rule+1032 (the rule at 0x1000)
    stubs = {'sub_' + ev: 'if(ctx.r5.u32==0x1000+980){store(ctx.r4.u32,DL);store(ctx.r4.u32+4,DH);}else if(ctx.r5.u32==0x1000+1032){store(ctx.r4.u32,IL);store(ctx.r4.u32+4,IH);}'
                          'else{printf("bad eval %08x' + NL + '",ctx.r5.u32);exit(1);}'}
    flat = sum(cs, [])
    vecs = ''.join('store(0x%08X,0x%08Xu);' % (ad + 4 * i, v) for ad, vs in VECS.items() for i, v in enumerate(vs))
    body = ('static const uint32_t C[]={' + ','.join('0x%08xu' % v for v in flat) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'size_t k=0;while(k<sizeof(C)/4){memset(objects,0,sizeof(objects));const uint32_t*q=&C[k];uint32_t np=q[22];' +
            'printf("CASE");for(int i=0;i<23;i++)printf(i==13||i==14?" g%u":" %08x",q[i]);printf("' + NL + '");' + vecs +
            # the rule at 0x1000 (to +1100), the instance at 0x1480 (+512 -> 0x1700), the list's entries at 0x1800 (96 each, at most 3)
            # the particle at 0x1A00 (to +260), the generator at 0x1C00
            'store(0x1000+280,q[2]);store(0x1000+284,q[3]);pointer(0x1000+307)[0]=uint8_t(q[4]);pointer(0x1000+1029)[0]=uint8_t(q[8]);pointer(0x1000+1081)[0]=uint8_t(q[12]);'
            'DL=q[5];DH=q[6];IL=q[9];IH=q[10];'
            'store(0x1480+512,0x1700);store(0x1700,0x1800);store16(0x1704,uint16_t(np));' +
            # the two random bytes: +138 (VelDmp's weight) = 7, +137 (ImpVar's) = 9; the table's places 7 and 9
            'pointer(0x1A00+138)[0]=7;pointer(0x1A00+137)[0]=9;' + ('store(0x%08X+4*7,q[7]);store(0x%08X+4*9,q[11]);' % (TABLE, TABLE)) +
            ('store(0x%08X,0x1C00);' % GEN) + 'store(0x1C00,q[13]);store(0x1C04,q[14]);' +
            'for(int i=0;i<3;i++){store(0x1A00+192+4*i,q[15+i]);store(0x1A00+224+4*i,q[18+i]);}store(0x1A00+256,q[21]);' +
            'k+=23;for(uint32_t p=0;p<np;p++){const uint32_t*e=&C[k];printf("POLY");for(int i=0;i<16;i++)printf(" %08x",e[i]);printf("' + NL + '");'
            'uint32_t b=0x1800+96*p;for(int j=0;j<4;j++)for(int i=0;i<3;i++)store(b+16*j+4*i,e[3*j+i]);for(int i=0;i<3;i++)store(b+64+4*i,e[12+i]);store(b+80,e[15]);k+=16;}' +
            'float g[2];memcpy(&g[0],&q[0],4);memcpy(&g[1],&q[1],4);'
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1480;ctx.r5.u32=0x1A00;ctx.r8.u32=0x77;ctx.f1.f64=g[0];ctx.f2.f64=g[1];' +
            ('sub_%s(ctx,nullptr);' % fn) +
            'printf("OUT %u",ctx.r3.u32&0xFF);for(int i=0;i<3;i++)printf(" %08x",load(0x1A00+192+4*i));for(int i=0;i<3;i++)printf(" %08x",load(0x1A00+224+4*i));'
            'printf(" %08x %08x g%u g%u' + NL + '",load(0x1A00+256),load(0x1A00+132),load(0x1C00),load(0x1C04));}}' + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'particle_collide', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs) and out.count('OUT') == len(cs), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    hits = sum(1 for l in out.split(nl) if l.startswith('OUT 1'))
    print(len(cs), 'cases', hits, 'hit')


if __name__ == '__main__':
    main()
