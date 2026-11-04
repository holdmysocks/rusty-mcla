"""mcCarStuck's update F-72170 run offline in its armed state and its righting: the generated code whole and unchanged in
vector mode, with its push-out F-51211, its reset F-80245 (and what that calls) and the normalise F-41334 in the
closure, on flat guest memory. The start-up vector constants the image holds as zero are filled in as the running
game has them: 0x828609E0 = +inf, 0x828608C0 = 0 (docs/reference/ai/raceline.md). The tick (the image's word at lis -32131 / 29952 + 8) is written per case.

Stubbed, as inputs of the case: the "driving" test F-65255 (the case's answer), the chassis's vfunc 11 (the case's
answer), the car instance getter F-71293 (an instance whose +28 is a marker word), the ball probe F-87651 (per call the
case's hit: its record's normal at +32 and depth at +68; the place, the radius and the include and state words it is
asked with are kept) and the gear setter F-59013 (its argument kept). The collider's vfuncs 2 and 3 (the reset from the
instance and the freeze) only mark that they ran: what they do is the port's frozen tick and set_pose path.

Object S at 0x1000 (vptr 0x1E00: slot 2 the reset F-80245), tune at 0x1100 (the defaults every car carries), chassis at
0x1200 (vptr 0x1E40: slot 11; +20 the gearbox 0x1400, +128 the collider 0x1500, +592 the flags), collider 0x1500 (vptr
0x1E80; +16 the instance 0x1700, +208 its matrix, +256 the body's place), instance matrix at 0x1700 + 16.

Writes <out-dir>/stuck_flip_ref.txt, one case a line: the case's words (state, timer, cooldown, saved place 3, matrix
16, flags, driving, vf11, gear, dt, three probes of hit + normal 3 + depth) '->' state, timer, cooldown, the instance
rows 0..3 (xyz each), SetGear's argument (ffffffff if not called), the collider calls (bit 0 freeze, bit 1 reset),
then the three probes' places (xyz) and radii (all hex)."""
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
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic const uint32_t *K;static uint32_t CALLS,GEAR,PROBE;\n'
                         'static void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

TICK = ((-32131 << 16) + 29952 + 8) & 0xFFFFFFFF
# the defaults of vehStuck on every car: TimeThresh, PosThresh, MoveThresh, TurnAmount, RotAmount, TransAmount, TimeFlipThresh,
# MinDropRetriggerTime (tune +24 ..)
TUNE = (0.3, 3.5, 5.0, 1.05, 0.0, 0.3, 1.0, 5.0)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def rot(rnd):
    # a random rotation's rows, near upright, on its side or on its roof
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(x * x for x in q))
    w, x, y, z = (v / n for v in q)
    m = [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)],
         [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
         [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]
    if rnd.random() < 0.15:
        # the car's back along the world's up: no right vector to make
        s = rnd.choice((1.0, -1.0))
        m = [[1.0, 0.0, 0.0], [0.0, 0.0, -s], [0.0, s, 0.0]]
    return m


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    upd, reset, driving, inst, probe, setgear = (addr(a.tool, f) for f in ('F-72170', 'F-80245', 'F-65255', 'F-71293', 'F-87651', 'F-59013'))
    rnd = random.Random(72170)
    cases = []
    for _ in range(a.cases):
        dt = rnd.choice((1 / 60, 1 / 60, 1 / 30, rnd.uniform(0.001, 0.05)))
        state = rnd.choice((0, 1, 1, 1, 4, 4))
        timer = rnd.choice((rnd.uniform(0, 1.2), 1.0 - dt, f32(1.0 - f32(dt)), 0.3 - dt, 1.0, 0.99, rnd.uniform(0.9, 1.0)))
        cool = rnd.choice((0.0, 0.0, -0.01, f32(dt), f32(dt) * 0.5, rnd.uniform(0, 5), 5.0))
        pos = [rnd.uniform(-2000, 2000), rnd.uniform(-10, 80), rnd.uniform(-2000, 2000)]
        r = rnd.choice((rnd.uniform(0, 6), 3.5, 3.4999, 3.5001, 5.0, rnd.uniform(0, 0.5)))
        d = [rnd.gauss(0, 1) for _ in range(3)]
        dn = math.sqrt(sum(x * x for x in d))
        saved = [f32(pos[k] + r * d[k] / dn) for k in range(3)]
        m = rot(rnd)
        if rnd.random() < 0.2:
            # up at about half way
            m[1][1] = rnd.choice((0.5, 0.49999997, 0.50000006))
        flags = rnd.choice((0, 0, 0, 0x10, 0x20, 0x30, 0x08, 0x28, 0x40))
        drv = int(rnd.random() < 0.15)
        vf11 = int(rnd.random() < 0.8)
        gear = rnd.choice((0, 1, 2, 2, 3, 5))
        probes = []
        for _ in range(3):
            hit = int(rnd.random() < 0.45)
            n = [rnd.gauss(0, 1) for _ in range(3)]
            if rnd.random() < 0.1:
                n = [rnd.uniform(-1e-3, 1e-3), -1.0, rnd.uniform(-1e-3, 1e-3)]
            nn = math.sqrt(sum(x * x for x in n))
            n = [f32(x / nn) for x in n]
            probes += [hit] + [fb(x) for x in n] + [fb(f32(rnd.uniform(0, 1.2)))]
        mat = []
        for k in range(3):
            mat += [fb(f32(x)) for x in m[k]] + [0]
        mat += [fb(f32(x)) for x in pos] + [0]
        cases.append([state, fb(f32(timer)), fb(f32(cool))] + [fb(x) for x in saved] + mat +
                     [flags, drv, vf11, gear, fb(f32(dt))] + probes)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x9002)sub_%s(ctx,base);else if(a==0x9011)ctx.r3.u32=K[24];'
            'else if(a==0x9022)CALLS|=2;else if(a==0x9023)CALLS|=1;'
            'else throw std::runtime_error("unexpected indirect call");}' % reset)
    body = ('static const uint32_t C[][%d]={' % len(cases[0]) + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));K=c;CALLS=0;GEAR=0xFFFFFFFF;PROBE=0;'
            'store(0x1E00+8,0x9002);store(0x1E40+44,0x9011);store(0x1E80+8,0x9022);store(0x1E80+12,0x9023);'
            'store(0x1000,0x1E00);store(0x1004,0x1100);store(0x1008,0x1200);store(0x1000+16,c[1]);'
            'store(0x1000+32,c[3]);store(0x1000+36,c[4]);store(0x1000+40,c[5]);'
            'store(0x1000+48,0x41440000);store(0x1000+52,0x41c80000);store(0x1000+56,c[0]);store(0x1000+64,0x1300);'
            'store(0x1000+68,c[2]);' +
            ''.join('store(0x1100+%d,0x%08x);' % (24 + 4 * k, fb(v)) for k, v in enumerate(TUNE)) +
            'store(0x1200,0x1E40);store(0x1200+20,0x1400);store(0x1200+128,0x1500);store(0x1200+592,c[22]);'
            'store(0x1400+20,c[25]);store(0x1500,0x1E80);store(0x1500+16,0x1700);'
            'for(int k=0;k<16;k++)store(0x1500+208+4*k,c[6+k]);store(0x1800+28,0x4d41524b);'
            'store(0x%08x,c[26]);' % TICK +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);' % upd)
    body += ('printf("%%08x %%08x %%08x",load(0x1000+56),load(0x1000+16),load(0x1000+68));'
             'for(int k=0;k<4;k++)printf(" %%08x %%08x %%08x",load(0x1710+16*k),load(0x1714+16*k),load(0x1718+16*k));'
             'printf(" %%08x %%x",GEAR,CALLS);for(int k=0;k<3;k++)printf(" %%08x %%08x %%08x %%08x %%08x %%08x",'
             'load(0x2000+32*k),load(0x2004+32*k),load(0x2008+32*k),load(0x200C+32*k),load(0x2010+32*k),load(0x2014+32*k));'
             'printf("%s");}' % NL) + nl
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
                'for(int k=0;k<4;k++){store(0x828609E0+4*k,0x7F800000);store(0x828608C0+4*k,0);}' + nl + body + '}' + nl)
    # (the probe: place, radius, include, states kept at 0x2000 + 32 k; the case's record written)
    probe_body = ('{uint32_t k=PROBE++;uint32_t o=0x2000+32*k;store(o,load(ctx.r4.u32));store(o+4,load(ctx.r4.u32+4));'
                  'store(o+8,load(ctx.r4.u32+8));float f=float(ctx.f1.f64);uint32_t b;memcpy(&b,&f,4);store(o+12,b);'
                  'store(o+16,ctx.r8.u32);store(o+20,ctx.r10.u32);const uint32_t*p=K+27+5*k;'
                  'if(p[0]){store(ctx.r6.u32+32,p[1]);store(ctx.r6.u32+36,p[2]);store(ctx.r6.u32+40,p[3]);store(ctx.r6.u32+68,p[4]);}'
                  'ctx.r3.u32=p[0];}')
    stubs = {
        'sub_' + driving: 'ctx.r3.u32=K[23];',
        'sub_' + inst: 'ctx.r3.u32=0x1800;',
        'sub_' + probe: probe_body,
        'sub_' + setgear: 'GEAR=ctx.r4.u32;',
    }
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + upd, 'sub_' + reset], main_src, a.out_dir, 'stuck_flip', stubs=stubs, vector=True, scratch_end=0x3000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    for l in lines:
        for wd in l.split():
            assert not (len(wd) == 8 and wd.startswith('82')), l
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'stuck_flip_ref.txt').write_text(nl.join(lines) + nl)
    st = [int(o.split()[0], 16) for o in out]
    print(len(cases), 'cases; to state 4', sum(c[0] == 1 and s == 4 for c, s in zip(cases, st)), '; righted',
          sum(int(o.split()[16], 16) & 1 for o in out), '; set gear', sum(o.split()[15] != 'ffffffff' for o in out),
          '; probes hit', sum(c[27 + 5 * k] for c in cases for k in range(3)))


if __name__ == '__main__':
    main()
