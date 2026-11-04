"""The arc-line-arc rail mcRailAmbientTCP run offline: its builder F-85722 (with the solver F-13911), the vtable's point
and tangent thunks F-88723 / F-31489 (F-56929 point, F-28568 tangent), the phase advance F-70353, the length F-62306,
the largest single turn F-39283 and the total turn F-65534 - the generated code whole and unchanged in vector mode,
with the rotation about y (F-57591), the 3D angle (F-88594) and the guest runtime's sin / cos / acos / asin in the
closure, on flat guest memory. The flush mode is the SDK's (FTZ and DAZ on the host while the code has it on). The
start-up vector constants the image holds as zero are filled in as the running game has them (+inf for the
normalisations' guard; docs/reference/ai/raceline.md); the zero-vector fallback stays zero.
Guest memory: the rail record 0x1000 (zeroed for every case, flags byte 0), inputs P 0x1100, H0 0x1110, T 0x1120,
H1 0x1130 (w lanes 0), outputs 0x1200 (point), 0x1210 (tangent), 0x1220 (lateral).
Cases: start / end points and headings in the city's range (headings unit, scaled, tilted or zero), radii 0.1 - 30 m,
the end point inside / on the start circle, |c2 - c1| about 2R for opposite senses, coincident circles (a zero-length
line), equal and opposite senses, height between P and T. Phases per case: 5, from a menu (0, -0, 1, 1 - ulp,
1 + ulp, 1.5, -0.25, uniform) and the two segment boundaries (the arc-1 end and the line end over the length, as
F-56929 computes them) moved -2 .. +2 float steps.
Writes <out-dir>/tcp_rail_ref.txt, one case a line: P H0 T H1 (xyz each), R, s0, s1, the 5 phase codes (a float, or
fff000bo: boundary b, offset o - 2), the 2 advance steps; then '->' valid, A B c1 c2 (xyz), a1, a2, the flags byte,
length, largest turn, total turn; then for a valid rail per phase: the phase used, point xyz, tangent xyz; then the two
advances (phase 0 by step 0, phase 3 by step 1). Floats as hex bits; w lanes are not compared (c1 / c2 keep stack
leftovers there)."""
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
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    # (the SDK's flush mode: FTZ | DAZ in MXCSR)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += 'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
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
    fids = ('F-85722', 'F-88723', 'F-31489', 'F-70353', 'F-62306', 'F-39283', 'F-65534')
    build_, pt_a, pt_b, adv, length, maxt, tot = (addr(a.tool, f) for f in fids)
    rnd = random.Random(85722)

    def heading():
        u = rnd.random()
        if u < 0.04:
            return [0.0, 0.0, 0.0]
        y = rnd.uniform(-math.pi, math.pi)
        h = [math.sin(y), 0.0, -math.cos(y)]
        if u < 0.15:
            h[1] = rnd.uniform(-0.3, 0.3)
        elif u < 0.25:
            k = rnd.choice((0.5, 2.0, rnd.uniform(0.01, 30)))
            h = [x * k for x in h]
        elif u < 0.3:
            h = [round(x) for x in h]
        return h

    def perp(h, s):
        # the centre's side: (-h.z, 0, h.x) made unit, kept with the sense bit set, else negated
        l = math.hypot(h[0], h[2])
        if l == 0:
            return [0.0, 0.0, 0.0]
        n = [-h[2] / l, 0.0, h[0] / l]
        return n if s else [-x for x in n]
    cases = []
    for _ in range(2000):
        R = rnd.choice((0.1, 0.4, 2.0, 5.0, rnd.uniform(0.05, 30), rnd.uniform(0.05, 30)))
        P = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        H0 = heading()
        s0, s1 = rnd.getrandbits(1), rnd.getrandbits(1)
        H1 = heading()
        n0 = perp(H0, s0)
        c1 = [P[i] + R * n0[i] for i in range(3)]
        dy = rnd.choice((0.0, 0.0, rnd.uniform(-6, 6)))
        u = rnd.random()
        if u < 0.1:
            # inside or on the start circle
            ang = rnd.uniform(-math.pi, math.pi)
            r = R * rnd.choice((rnd.random(), 0.999999, 1.0, 1.000001))
            T = [c1[0] + r * math.cos(ang), P[1] + dy, c1[2] + r * math.sin(ang)]
        elif u < 0.3:
            # the end circle at about 2R from the start circle (opposite senses' limit)
            s1 = 1 - s0 if rnd.random() < 0.8 else s1
            ang = rnd.uniform(-math.pi, math.pi)
            d = 2 * R * rnd.choice((1.0, 1.0000001, 0.9999999, 1.00001, 0.99999, rnd.uniform(0.9, 1.3)))
            c2 = [c1[0] + d * math.cos(ang), 0.0, c1[2] + d * math.sin(ang)]
            n1 = perp(H1, s1)
            T = [c2[0] - R * n1[0], P[1] + dy, c2[2] - R * n1[2]]
        elif u < 0.4:
            # coincident circles: the end on the start circle, its heading the circle's tangent, the same sense
            s1 = s0
            ang = rnd.uniform(-math.pi, math.pi)
            T = [c1[0] + R * math.cos(ang), P[1] + dy, c1[2] + R * math.sin(ang)]
            rad = [math.cos(ang), 0.0, math.sin(ang)]
            # perp(H1, s1) must be -rad: H1 = (rad.z, 0, -rad.x) or its opposite
            H1 = [rad[2], 0.0, -rad[0]] if s1 else [-rad[2], 0.0, rad[0]]
            if rnd.random() < 0.5:
                H1 = [x + rnd.uniform(-1e-6, 1e-6) for x in H1]
        elif u < 0.45:
            T, H1 = list(P), list(H0)
        else:
            dist = rnd.choice((rnd.uniform(0, 3), rnd.uniform(0, 30), rnd.uniform(0, 150)))
            ang = rnd.uniform(-math.pi, math.pi)
            T = [P[0] + dist * math.cos(ang), P[1] + dy, P[2] + dist * math.sin(ang)]
        menu = (0.0, -0.0, 1.0, f32(1 - 2 ** -24), f32(1 + 2 ** -23), 1.5, -0.25)
        ph = [fb(rnd.choice(menu)), fb(rnd.random()), fb(rnd.choice(menu + (rnd.random(),)))]
        ph += [0xFFF00000 | rnd.choice((1, 2)) << 4 | rnd.randint(0, 4) for _ in range(2)]
        rnd.shuffle(ph)
        steps = [fb(rnd.choice((0.0, rnd.uniform(-3, 3), rnd.uniform(0, 30), 1e-3))) for _ in range(2)]
        cases.append([fb(f32(x)) for x in P + H0 + T + H1 + [R]] + [s0, s1] + ph + steps)
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    pr = 'auto P=[](uint32_t a){printf(" %08x",load(a));};'
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);' + pr + nl +
            'auto bf=[](uint32_t u){float f;memcpy(&f,&u,4);return f;};auto fbits=[](float f){uint32_t u;memcpy(&u,&f,4);return u;};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));'
            'for(int i=0;i<4;i++)for(int k=0;k<3;k++)store(0x1100+16*i+4*k,c[3*i+k]);'
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r5.u32=0x1110;ctx.r6.u32=c[13];'
            'ctx.r7.u32=0x1120;ctx.r8.u32=0x1130;ctx.r9.u32=c[14];ctx.f1.f64=bf(c[12]);'
            'sub_@BUILD(ctx,nullptr);int valid=int(ctx.r3.u32);printf("%d",valid);'
            'for(int o:{0,4,8,16,20,24,48,52,56,64,68,72,32,36})P(0x1000+o);printf(" %02x",*pointer(0x1000+160));'
            '{Context x{};x.r1.u32=0x2F00;x.r3.u32=0x1000;sub_@LEN(x,nullptr);printf(" %08x",fbits(float(x.f1.f64)));}'
            '{Context x{};x.r1.u32=0x2F00;x.r3.u32=0x1000;sub_@MAX(x,nullptr);printf(" %08x",fbits(float(x.f1.f64)));}'
            '{Context x{};x.r1.u32=0x2F00;x.r3.u32=0x1000;sub_@TOT(x,nullptr);printf(" %08x",fbits(float(x.f1.f64)));}'
            'if(!valid){printf("@NL");continue;}'
            # the boundaries as F-56929 computes them
            'float R=getf(0x1000+80),a1r=std::fabs(getf(0x1000+32)*R),a2r=std::fabs(getf(0x1000+36)*R);'
            'float dx=getf(0x1000)-getf(0x1010),dy=getf(0x1004)-getf(0x1014),dz=getf(0x1008)-getf(0x1018);'
            'float ln=std::sqrt(float(std::fma(double(dx),double(dx),double(float(std::fma(double(dz),double(dz),double(dy*dy)))))));'
            'float L=(a2r+ln)+a1r;float bnd[3]={0,a1r,ln+a1r};float phs[5];' + nl +
            'for(int i=0;i<5;i++){uint32_t w=c[15+i];float ph;if((w&0xFFFFFF00u)==0xFFF00000u){float b=L>0?bnd[(w>>4)&3]/L:0;'
            'int off=int(w&15)-2;uint32_t u=fbits(b);ph=b==0?bf(off<0?0x80000000u|uint32_t(-off):uint32_t(off)):bf(u+off);}else ph=bf(w);phs[i]=ph;'
            'Context x{};x.r1.u32=0x2F00;x.r3.u32=0x1200;x.r4.u32=0x1000;x.f1.f64=ph;x.r7.u32=0x1220;x.r8.u32=0x1210;store(0x1220,0xDEADBEEF);'
            'sub_@PTB(x,nullptr);if(load(0x1220)!=0)throw std::runtime_error("lateral");'
            'uint32_t keep[6];for(int k=0;k<3;k++){keep[k]=load(0x1200+4*k);keep[3+k]=load(0x1210+4*k);}'
            'printf(" %08x",fbits(ph));for(int k=0;k<6;k++)printf(" %08x",keep[k]);'
            'Context y{};y.r1.u32=0x2F00;y.r3.u32=0x1200;y.r4.u32=0x1000;y.f1.f64=ph;y.r6.u32=0x1210;memset(objects+0x200,0,0x20);'
            'sub_@PTA(y,nullptr);for(int k=0;k<3;k++)if(load(0x1200+4*k)!=keep[k]||load(0x1210+4*k)!=keep[3+k])throw std::runtime_error("thunks differ");}' + nl +
            'for(int i=0;i<2;i++){Context x{};x.r1.u32=0x2F00;x.r3.u32=0x1000;x.f1.f64=phs[3*i];x.f2.f64=bf(c[20+i]);'
            'sub_@ADV(x,nullptr);printf(" %08x",fbits(float(x.f1.f64)));}'
            'printf("@NL");}' + nl)
    for k, v in (('@BUILD', build_), ('@LEN', length), ('@MAX', maxt), ('@TOT', tot), ('@PTA', pt_a), ('@PTB', pt_b), ('@ADV', adv), ('@NL', NL)):
        body = body.replace(k, v)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + n for n in (build_, pt_a, pt_b, adv, length, maxt, tot)], main_src, a.out_dir, 'tcp_rail', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join(('%d' % x) if i in (13, 14) else '%08x' % x for i, x in enumerate(c)) + ' -> ' + o for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'tcp_rail_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] == '1' for o in out), 'valid')


if __name__ == '__main__':
    main()
