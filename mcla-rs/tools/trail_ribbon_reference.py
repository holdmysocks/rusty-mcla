"""A light trail's ribbon (F-80988, called by the trail draw F-89089 with f1 = 0.75) run offline: the whole function,
unchanged generated code on flat guest memory in vector mode, with the C library's pow for its pow callee F-21955 (the CRT's, not the game's). The screen projection callee F-25309
is a stub that computes x m0 + y m4 + z m8 + m12 (a row, column-major m) and / w, as the port's test does; the case's m is
a 1280 x 720 viewport times a projection of random lens. The first point's screen place is a stack slot no code writes
(docs/LIGHTING.md "Light trails"): it is filled here with the stub's projection of the first point, as the port takes it.
The start-up vector constants the image holds as zero are filled in as the running game has them (0x828609E0 +inf).
Guest memory: the record 0x1000 (+112 the eight points xyzw, +240 their frames, +272 their values, +304 the count,
+324 the width), the outputs: centres 0x1200, across 0x1280, alphas 0x1300 (8 floats), the width splat 0x1320, the
scratch 0x1330, start cap 0x1340, end cap 0x1350; stack 0x2F00.
Writes <out-dir>/trail_ribbon_ref.txt, one record a line: count, frame, width, m (16), eight (x y z frame value), then "->"
the point count, the loop's own count, start, end, the points (centre, across) and the loop's alphas; floats as hex bits."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += 'static const uint32_t*cur;' + chr(10)

# (the recomp is built with -msse4.1: simde's dot product is the native one, as in the game)
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    rib, proj, pw = (addr(a.tool, f) for f in ('F-80988', 'F-25309', 'F-21955'))
    rnd = random.Random(53)
    cases = []
    for _ in range(4000):
        n = rnd.choice((3, 4, 5, 6, 7, 8, 8, 8))
        frame = rnd.randint(100, 100000)
        width = f32(rnd.choice((27.0, 25.4, 23.1875, rnd.uniform(6, 40))))
        fy = rnd.uniform(2.0, 6.0)
        fx = fy * 9 / 16
        m = [f32(x) for x in (fx * 640, 0, 0, 0, 0, -fy * 360, 0, 0, -640, -360, -1, -1, 0, 0, -0.2, 0)]
        p = [rnd.uniform(-20, 20), rnd.uniform(-10, 10), -rnd.uniform(2, 300)]
        step = -p[2] * rnd.choice((0.0002, 0.002, 0.006, 0.006, 0.02, 0.02, 0.05))
        pts, stamps, vals = [], [], []
        st = frame - rnd.choice((0, 0, 1, 3))
        for k in range(8):
            pts.append([f32(x) for x in p])
            stamps.append(st)
            vals.append(f32(rnd.choice((0.5, 0.55, rnd.uniform(0, 0.55), 0.0))))
            u = rnd.random()
            s = step * (rnd.choice((0.01, 1.0, 5.0)) if u < 0.3 else 1.0)
            p = [p[0] + rnd.gauss(0, s), p[1] + rnd.gauss(0, s), p[2] + rnd.gauss(0, s * 0.3)]
            st -= rnd.choice((1, 1, 1, 2))
        cases.append((n, frame, width, m, pts, stamps, vals))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = []
    for n, frame, width, m, pts, stamps, vals in cases:
        r = [n, frame, fb(width)] + [fb(x) for x in m]
        for k in range(8):
            r += [fb(pts[k][0]), fb(pts[k][1]), fb(pts[k][2]), stamps[k] & 0xFFFFFFFF, fb(vals[k])]
        rows.append('{%s},' % ','.join('0x%08x' % x for x in r))
    stub = ('auto flt=[](uint32_t u){float f;memcpy(&f,&u,4);return f;};auto bits=[](float f){uint32_t u;memcpy(&u,&f,4);return u;};'
            'float x=flt(load(ctx.r4.u32)),y=flt(load(ctx.r4.u32+4)),z=flt(load(ctx.r4.u32+8));float m[16];for(int i=0;i<16;i++)m[i]=flt(cur[3+i]);'
            'auto row=[&](int c){float v=x*m[c];v=v+y*m[4+c];v=v+z*m[8+c];return v+m[12+c];};float w=row(3);'
            'store(ctx.r5.u32,bits(row(0)/w));store(ctx.r6.u32,bits(row(1)/w));')
    setup = ('memset(objects,0,0x2000);'
             'for(int k=0;k<8;k++){for(int c=0;c<3;c++)store(0x1000+112+16*k+4*c,cur[19+5*k+c]);store(0x1000+112+16*k+12,0x3f800000);'
             'store(0x1000+240+4*k,cur[19+5*k+3]);store(0x1000+272+4*k,cur[19+5*k+4]);}'
             'store(0x1000+304,cur[0]);store(0x1000+324,cur[2]);'
             'for(int k=0;k<4;k++)store(0x1320+4*k,cur[2]);'
             'for(int k=0;k<8;k++)store(0x1300+4*k,0xDEADBEEF);for(int k=0;k<8;k++){store(0x1340+4*k,0xDEADBEEF);}'
             'put(0x827D2DCC,cur[1]);store(0x2F00+84,0x1350);'
             # (the stack slot of the first point's screen place: the stub's projection of the first point)
             '{Context t{};t.r4.u32=0x1000+112;t.r5.u32=0x2F00-496+96;t.r6.u32=0x2F00-496+100;sub_%s(t,nullptr);}' % proj)
    body = ('static const uint32_t C[][59]={' + nl + nl.join(rows) + nl + '};' + nl +
            'for(auto&c:C){cur=c;' + setup + 'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;ctx.r5.u32=0x1280;ctx.r6.u32=0x1300;'
            'ctx.r7.u32=0x1320;ctx.r8.u32=0x1330;ctx.r10.u32=0x1340;ctx.r1.u32=0x2F00;ctx.f1.f64=0.75;'
            'sub_%s(ctx,nullptr);int k=int(ctx.r3.u32);int real=0;for(int j=0;j<8;j++)if(load(0x1300+4*j)!=0xDEADBEEF)real++;' % rib +
            'printf("%d %d",k,real);for(int j=0;j<3;j++)printf(" %08x",load(0x1340+4*j));for(int j=0;j<3;j++)printf(" %08x",load(0x1350+4*j));'
            'for(int i=0;i<k;i++){for(int j=0;j<3;j++)printf(" %08x",load(0x1200+16*i+4*j));for(int j=0;j<3;j++)printf(" %08x",load(0x1280+16*i+4*j));}'
            'for(int i=0;i<real;i++)printf(" %08x",load(0x1300+4*i));' + ('printf("%s");}' % NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
                'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
                'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + rib], main_src, a.out_dir, 'trail_ribbon', stubs={'sub_' + proj: stub, 'sub_' + pw: 'ctx.f1.f64=pow(ctx.f1.f64,ctx.f2.f64);'}, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (n, frame, width, m, pts, stamps, vals), o in zip(cases, out):
        s = '%d %d %08x %s' % (n, frame, fb(width), ' '.join('%08x' % fb(x) for x in m))
        for k in range(8):
            s += ' %08x %08x %08x %d %08x' % (fb(pts[k][0]), fb(pts[k][1]), fb(pts[k][2]), stamps[k], fb(vals[k]))
        lines.append(s + ' -> ' + o)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'trail_ribbon_ref.txt').write_text(nl.join(lines) + nl)
    ks = [int(o.split()[0]) for o in out]
    print(len(lines), 'records, points out', {k: ks.count(k) for k in sorted(set(ks))})


if __name__ == '__main__':
    main()
