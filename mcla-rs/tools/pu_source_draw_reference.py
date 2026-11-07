"""The power-up source draw (F-12500) run offline with the real sphere test (F-50958): unchanged generated code on flat
guest memory (recomp_closure, vector shim, -msse4.1). F-12500(matrix r3, look r4, f1, f2 the sphere's radius, r8 the
ground glow): the camera distance fade from the viewport's +112 (place), the type 0 .. 23 with a look in the table
(every entry is set by the draw init, so all 24 are set here), F-50958 on the matrix's place (+48) with radius f2 against
the viewport's planes (+864 .. +976, two blocks of four planes as x / y / z / w rows), then the card F-11753 and with r8 the
glow F-50763. Stubs: F-11753 logs " C:<r4>:<f1>:<f2>:<f3>", F-50763 logs " G:<r4>:<f1>:<f2>:<f3>" (float bits), the
render state call before them does nothing (it sets a technique only).
The planes: six per case (F-53768's order: +768, +784, +800, +816, +832, +848), laid into the two blocks as F-53768's tail
copies them: block 1 = (P0, P1, P2, P2), block 2 = (P3, P4, P5, P5). Each plane is put about the sphere: its distance
from the centre a random value in [-3r, 3r], or -r / r exactly as far as float allows, or far inside.
A case: kind, f1, radius, glow flag, place (3), camera (3), planes (24), all float bits but kind and flag.
Out per case: the case words, then the calls in order. Test `pu_draw::tests::source_draw_matches_the_original`.
Usage: pu_source_draw_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --tool mcla-tool
       --work DIR --out FILE"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

recomp_closure.EXTRA += '#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
_vector_shim = recomp_closure.vector_shim


def _shim():
    # the condition register's vector compare (the SDK's Condition::setFromMask) and its summary-overflow bit, as
    # arm_towards_reference.py
    s = _vector_shim()
    old = 'struct Condition {\n    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    return s.replace(old, '#include <simde/x86/sse4.1.h>\nstruct Condition {\n    bool so{}, eq{}, lt{}, gt{};\n'
                     '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)


recomp_closure.vector_shim = _shim

NCASE = 34


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--tool', required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    a = ap.parse_args()
    draw, sphere, card, glow = (addr(a.tool, f) for f in ('F-12500', 'F-50958', 'F-11753', 'F-50763'))
    rnd = random.Random(20261107)
    cases = []
    for i in range(4000):
        kind = rnd.randrange(24) if rnd.random() < 0.95 else rnd.randrange(24, 28)
        lift = rnd.choice((0.0, -0.5))
        r = rnd.choice((1.0, 0.25, 0.25, 1.0, rnd.uniform(0.01, 5.0), 0.0))
        g = int(rnd.random() < 0.6)
        p = [f32(rnd.uniform(-2500, 2500)), f32(rnd.uniform(-20, 120)), f32(rnd.uniform(-2500, 2500))]
        # the camera: near the place, or up to 400 m away (the fade ends at 250 m)
        cam = [f32(p[k] + rnd.uniform(-1, 1) * rnd.choice((2.0, 40.0, 130.0, 260.0, 400.0))) for k in range(3)]
        planes = []
        for k in range(6):
            n = [rnd.gauss(0, 1) for _ in range(3)]
            ln = math.sqrt(sum(x * x for x in n)) or 1.0
            s = rnd.choice((1.0, 1.0, rnd.uniform(0.2, 3.0)))
            n = [f32(x / ln * s) for x in n]
            t = rnd.choice((rnd.uniform(-3, 3) * max(r, 0.1), -r, r, rnd.uniform(5, 200), rnd.uniform(5, 200)))
            # w so that x*X + (y*Y + (z*Z + w)) is about t
            w = f32(t - (p[0] * n[0] + p[1] * n[1] + p[2] * n[2]))
            planes.append(n + [w])
        c = [kind, fb(lift), fb(r), g] + [fb(x) for x in p] + [fb(x) for x in cam] + [fb(x) for pl in planes for x in pl]
        assert len(c) == NCASE
        cases.append(c)
    # (a NaN place and a zero-length plane case)
    cases[0][4] = 0x7FC00000
    cases[1][10:14] = [0, 0, 0, 0]
    nl = chr(10)
    NL = chr(92) + 'n'
    F = '[](double v){float f=float(v);uint32_t b;std::memcpy(&b,&f,4);return b;}'
    stubs = {
        'sub_' + card: 'printf(" C:%%u:%%08x:%%08x:%%08x",ctx.r4.u32,%s(ctx.f1.f64),%s(ctx.f2.f64),%s(ctx.f3.f64));' % (F, F, F),
        'sub_' + glow: 'printf(" G:%%u:%%08x:%%08x:%%08x",ctx.r4.u32,%s(ctx.f1.f64),%s(ctx.f2.f64),%s(ctx.f3.f64));' % (F, F, F),
    }
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    disp = 'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){printf(" V");ctx.r3.u32=0;}'
    # the viewport's two plane blocks: lane j of block b is plane L[b][j]; rows x, y, z, w at +0, +16, +32, +48
    lay = ''.join('store(0x1200+%d+%d*16+4*%d,c[10+4*%d+%d]);' % (864 + 64 * b, comp, j, P, comp)
                  for b, lanes in enumerate(((0, 1, 2, 2), (3, 4, 5, 5))) for j, P in enumerate(lanes) for comp in range(4))
    body = ('static const uint32_t C[][%d]={' % NCASE + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'for(int k=0;k<%d;k++)printf(k?" %%08x":"%%08x",c[k]);' % NCASE +
            'store(0x82839704,0x1200);for(int k=0;k<24;k++)store(0x82907428+4*k,0x1800);' +
            'store(0x1000,0x3f800000);store(0x1000+20,0x3f800000);store(0x1000+40,0x3f800000);' +
            'store(0x1000+48,c[4]);store(0x1000+52,c[5]);store(0x1000+56,c[6]);store(0x1000+60,0x3f800000);' +
            'store(0x1200+112,c[7]);store(0x1200+116,c[8]);store(0x1200+120,c[9]);' + lay +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=c[0];ctx.r8.u32=c[3];ctx.r1.u32=0x2F00;' +
            'float f;std::memcpy(&f,&c[1],4);ctx.f1.f64=f;std::memcpy(&f,&c[2],4);ctx.f2.f64=f;' +
            'try{sub_%s(ctx,nullptr);}catch(std::exception&){printf(" X");}printf("' % draw + NL + '");}' + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    # (the render state call: a technique set, nothing the decision reads)
    import re
    src = recomp_closure.extract(idx['sub_' + draw], 'sub_' + draw)
    others = [c for c in set(re.findall(r'\b(sub_[0-9A-F]{8})\(ctx, base\)', src)) if c not in ('sub_' + sphere, 'sub_' + card, 'sub_' + glow)]
    assert len(others) == 1, others
    stubs[others[0]] = ''
    exe = build(idx, ['sub_' + draw], main_src, a.work, 'pu_source_draw', stubs=stubs, vector=True, flags=('-msse4.1',))
    out = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, check=True).stdout.replace('\r', '')
    lines = out.splitlines()
    assert len(lines) == len(cases), len(lines)
    assert not any(' X' in l or ' V' in l for l in lines)
    a.out.write_text(out)
    print(len(lines), 'lines')


if __name__ == '__main__':
    main()
