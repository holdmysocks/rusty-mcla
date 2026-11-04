"""A traffic car's drawn steer (F-37033, the car's render update, with its 8-sample ring F-17429) run offline: the whole
update, unchanged generated code on flat guest memory in vector mode, up to its first call after the ring (the car's
vtable slot 1, which ends the run). The rail's two calls are stubs that record what they are given and return the case's
values: slot 6 (the phase a distance on: F-77217, compared on its own elsewhere) and slot 10 (the curve's derivative there:
F-26650, the port's `LaneCurve::eval`). Callees that only work the tilt, the spin and the drawing are stubs; the ring and
the mean (f25) do not depend on them. The start-up vector constants the image holds as zero are filled in as the running
game has them (0x82860950 the xyz mask, 0x828609E0 +inf; docs/reference/ai/raceline.md).
Guest memory: car 0x1000 (+128 the instance matrix rows X, Y, Z, place; +448 its type 0x2200 (+112 byte, +120 wheel count 0);
+452 the behaviour 0x1E40 or 0 (+188 the rail 0x1F40 or 0, +208 the phase, +212 the curve speed); +500 byte 0; +3584 the
parked steer), the draw object 0x2000 (+28 0x2100: +16 the car, +20 the model 0x2120 with wheel_0's bone record at 0x2180,
+96 the hub; +32 the LOD; +88 the ring, kept from frame to frame of a sequence), the thread context 0x2280, the focus 0x2300.
Writes <out-dir>/drawn_steer_ref.txt, one frame per line: the inputs, then "->" slot 6's (f1 f2 f3) or "-", slot 10's f1 or
"-", the ring (count, next, sum, 8 values) and f25; floats as hex bits."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n')

# (the vector shim lacks a few things this function uses: the vector temporary's type and the condition register's
# setFromMask / so, written as the SDK's rex/ppc/context.h has them)
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim

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
    upd, ring = (addr(a.tool, f) for f in ('F-37033', 'F-17429'))
    keep = [addr(a.tool, f) for f in ('F-91983', 'F-50349')]
    rnd = random.Random(41)
    frames = []
    for seq in range(350):
        has_b = rnd.random() < 0.85
        has_rail = has_b and rnd.random() < 0.9
        t2 = rnd.choice((0, 0, 1))
        lod = rnd.choice((0, 2, 3, 4))
        park = f32(rnd.gauss(0, math.pi / 8)) if rnd.random() < 0.5 else 0.0
        hubz = f32(-rnd.uniform(1.0, 3.2))
        reset = 1 if rnd.random() < 0.8 else 0
        r = rnd.choice((44.999, 45.0, 45.001, rnd.uniform(0, 60), rnd.uniform(0, 44)))
        for k in range(rnd.randint(1, 13)):
            yaw = rnd.uniform(-math.pi, math.pi)
            pitch = rnd.uniform(-0.1, 0.1)
            fwd = [math.sin(yaw) * math.cos(pitch), math.sin(pitch), -math.cos(yaw) * math.cos(pitch)]
            pos = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
            ang = rnd.uniform(0, 2 * math.pi)
            focus = [pos[0] + r * math.cos(ang), pos[2] + r * math.sin(ang)]
            u = rnd.random()
            if u < 0.06:
                tan = [0.0, 0.0, 0.0]
            elif u < 0.1:
                tan = [0.0, rnd.uniform(-5, 5), 0.0]
            else:
                ty = yaw + rnd.uniform(-0.8, 0.8)
                s = rnd.choice((1.0, rnd.uniform(5, 400), rnd.uniform(1e-3, 1)))
                tan = [math.sin(ty) * s, rnd.uniform(-0.2, 0.2) * s, -math.cos(ty) * s]
            phase = rnd.choice((0.0, 1.0, rnd.random()))
            ds = rnd.choice((0.0, rnd.uniform(5, 400)))
            ret6 = rnd.choice((rnd.random(), 1.0, 1.0000001, rnd.uniform(1, 2), 0.99999994))
            dt = rnd.choice((1 / 30, 1 / 60, 0.05))
            frames.append([reset if k == 0 else 0, int(has_b), int(has_rail), t2, lod] +
                          [f32(x) for x in [park, hubz] + focus + pos + fwd + [phase, ds, ret6] + tan + [dt]])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % (x if i < 5 else fb(x)) for i, x in enumerate(f)) for f in frames)
    s6, s10, sv = 0x82FF0100, 0x82FF0200, 0x82FF0300
    disp = ('struct Stop{};static uint32_t g6[4],g10[2];static const uint32_t*cur;'
            'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'auto bits=[](double d){float f=float(d);uint32_t u;memcpy(&u,&f,4);return u;};'
            'auto flt=[](uint32_t u){float f;memcpy(&f,&u,4);return double(f);};'
            'if(a==0x%x){g6[0]=1;g6[1]=bits(ctx.f1.f64);g6[2]=bits(ctx.f2.f64);g6[3]=bits(ctx.f3.f64);ctx.f1.f64=flt(cur[17]);}'
            'else if(a==0x%x){g10[0]=1;g10[1]=bits(ctx.f1.f64);store(ctx.r3.u32,cur[18]);store(ctx.r3.u32+4,cur[19]);store(ctx.r3.u32+8,cur[20]);store(ctx.r3.u32+12,0);}'
            'else if(a==0x%x)throw Stop{};else throw std::runtime_error("unexpected indirect call");}' % (s6, s10, sv))
    setup = ('memset(objects,0,0x1000);memset(objects+0x1100,0,0x2000-0x1100);'  # (all but the ring's object 0x2000 .. 0x20FF)
             'for(int i=0x2000;i<0x2100;i++)if(i<0x2058||i>=0x2084)*pointer(i)=0;'
             'if(c[0]){for(int i=0x2058;i<0x2084;i++)*pointer(i)=0;}'
             'store(0x1000,0x2340);store(0x2340+4,0x%x);' % sv +
             'store(0x1000+128,0x3f800000);store(0x1000+148,0x3f800000);'
             'store(0x1000+160,c[12]^0x80000000u);store(0x1000+164,c[13]^0x80000000u);store(0x1000+168,c[14]^0x80000000u);'
             'store(0x1000+176,c[9]);store(0x1000+180,c[10]);store(0x1000+184,c[11]);'
             'store(0x1000+448,0x2200);*pointer(0x2200+112)=c[3];store(0x1000+452,c[1]?0x1E40:0);store(0x1000+3584,c[5]);'
             'store(0x1E40+188,c[2]?0x1F40:0);store(0x1E40+208,c[15]);store(0x1E40+212,c[16]);'
             'store(0x1F40,0x1F60);store(0x1F60+24,0x%x);store(0x1F60+40,0x%x);' % (s6, s10) +
             'store(0x2000+4,0x2380);store(0x2000+28,0x2100);store(0x2000+32,c[4]);store(0x2100+16,0x1000);store(0x2100+20,0x2120);'
             'store(0x2120+176,0x2180);store(0x2180+104,c[6]);'
             'store(0x2280,0x22A0);store(0x2300,c[7]);store(0x2308,c[8]);')
    body = ('static const uint32_t C[][22]={' + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'for(int k=0;k<3;k++)put(0x82860950+4*k,0xFFFFFFFF);put(0x8286095C,0);for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);' + nl +
            'for(auto&c:C){cur=c;' + setup + 'g6[0]=g10[0]=0;Context ctx{};ctx.r3.u32=0x2000;ctx.r4.u32=0x1000;ctx.r6.u32=0x2300;'
            'ctx.r13.u32=0x2280;ctx.r1.u32=0x2F00;{float d;memcpy(&d,&c[21],4);ctx.f1.f64=d;}'
            'try{sub_%s(ctx,nullptr);printf("noreach%s");continue;}catch(Stop&){}' % (upd, NL) +
            'uint32_t f25;{float f=float(ctx.f25.f64);memcpy(&f25,&f,4);}'
            'if(g6[0])printf("%08x %08x %08x ",g6[1],g6[2],g6[3]);else printf("- ");'
            'if(g10[0])printf("%08x ",g10[1]);else printf("- ");'
            'printf("%d %d",int(load(0x2000+88)),int(load(0x2000+92)));for(int i=0;i<9;i++)printf(" %08x",load(0x2000+96+4*i));' +
            ('printf(" %%08x%s",f25);}' % NL) + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    # (the instance matrix getter F-33765 -> the car's +128; the tilt, spin, wheel and drawing callees: empty)
    calls = [addr(a.tool, f) for f in ('F-33765', 'F-14882', 'F-70315', 'F-74033', 'F-97374', 'F-25963', 'F-52786', 'F-83846',
                                       'F-16906', 'F-14577', 'F-63589', 'F-72580')]
    stubs = {'sub_' + n: '' for n in calls}
    stubs['sub_' + calls[0]] = 'ctx.r3.u32=0x1080;'
    exe = build(idx, ['sub_' + upd, 'sub_' + ring] + ['sub_' + k for k in keep], main_src, a.out_dir, 'drawn_steer',
                stubs=stubs, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(frames), (len(out), len(frames))
    lines = ['%s -> %s' % (' '.join(('%d' % x) if i < 5 else '%08x' % fb(x) for i, x in enumerate(f)), o) for f, o in zip(frames, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'drawn_steer_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'frames,', sum(o.split()[0] != '-' for o in out), 'sampled,', sum('noreach' in o for o in out), 'not reached')


if __name__ == '__main__':
    main()
