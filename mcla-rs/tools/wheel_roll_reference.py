"""A traffic car's drawn wheel roll (F-37033, the car's render update) run offline: the whole update, unchanged generated
code on flat guest memory in vector mode, to its end; as tools/drawn_steer_reference.py (the ring and the steer f25 run
with it) but with 0 - 6 wheels (type +120), each with its hub (F-91983: wheel record +96, its y the roll's radius) and its
angle (draw object +64 + 4i; +36 + 4i its drawn matrix, written), the car's velocity (+208), a full instance matrix, the lock bit (F-70315, a stub returning
the case's byte), the guest's cos / sin (F-63589 / F-72580) in the closure. The car's vtable slot 1 returns a zeroed
collider. The tilt, compression and drawing callees are empty stubs (they write no angle).
Writes <out-dir>/wheel_roll_ref.txt, one frame per line: the inputs, then "->" f25 (taken at the car's slot 1 call right after the ring: the epilogue restores the caller's f25) and the wheel angles after (6, hex
bits; the ones past the count unchanged)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic const uint32_t*cur;static uint32_t g25;\n')

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
    rnd = random.Random(43)
    frames = []
    NI = 7  # (ints: ring reset, behaviour, rail, trailer byte, LOD, wheel count, lock byte)
    for seq in range(400):
        has_b = rnd.random() < 0.8
        has_rail = has_b and rnd.random() < 0.9
        t2 = rnd.choice((0, 0, 0, 1))
        lod = rnd.choice((0, 2, 3, 4))
        nw = rnd.choice((0, 2, 4, 4, 4, 6))
        park = f32(rnd.gauss(0, math.pi / 8)) if rnd.random() < 0.5 else 0.0
        hubz = f32(-rnd.uniform(1.0, 3.2))
        hy = [f32(rnd.choice((rnd.uniform(0.25, 0.6), rnd.uniform(0.25, 0.6), rnd.uniform(0.6, 1.2), 1e-3, -0.3))) for _ in range(6)]
        ang = [f32(rnd.choice((rnd.uniform(0, 2 * math.pi), 0.0, 1e-6, 2 * math.pi, 6.2831850, rnd.uniform(6.27, 6.2831)))) for _ in range(6)]
        reset = 1 if rnd.random() < 0.8 else 0
        r = rnd.choice((74.999, 75.0, 75.001, 44.999, 45.0, rnd.uniform(0, 90), rnd.uniform(0, 74), rnd.uniform(0, 30)))
        yaw = rnd.uniform(-math.pi, math.pi)
        for k in range(rnd.randint(1, 13)):
            yaw += rnd.uniform(-0.2, 0.2)
            pitch = rnd.uniform(-0.15, 0.15)
            roll = rnd.uniform(-0.1, 0.1)
            # (rows X, Y, Z of the instance matrix: yaw about y, then pitch about x, then roll about z)
            cy, sy, cp, sp, cr, sr = math.cos(yaw), math.sin(yaw), math.cos(pitch), math.sin(pitch), math.cos(roll), math.sin(roll)
            ry = [[cy, 0, -sy], [0, 1, 0], [sy, 0, cy]]
            rx = [[1, 0, 0], [0, cp, sp], [0, -sp, cp]]
            rz = [[cr, sr, 0], [-sr, cr, 0], [0, 0, 1]]

            def mm(A, B):
                return [[sum(A[i][t] * B[t][j] for t in range(3)) for j in range(3)] for i in range(3)]
            R = mm(mm(rz, rx), ry)
            fwd = [-x for x in R[2]]
            pos = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
            an = rnd.uniform(0, 2 * math.pi)
            focus = [pos[0] + r * math.cos(an), pos[2] + r * math.sin(an)]
            if rnd.random() < 0.06:
                tan = [0.0, 0.0, 0.0]
            else:
                ty = yaw + rnd.uniform(-0.8, 0.8)
                sc = rnd.choice((1.0, rnd.uniform(5, 400), rnd.uniform(1e-3, 1)))
                tan = [math.sin(ty) * sc, rnd.uniform(-0.2, 0.2) * sc, -math.cos(ty) * sc]
            phase = rnd.choice((0.0, 1.0, rnd.random()))
            ds = rnd.choice((0.0, rnd.uniform(5, 400)))
            ret6 = rnd.choice((rnd.random(), 1.0, rnd.uniform(1, 2)))
            dt = rnd.choice((1 / 30, 1 / 60, 0.05))
            w = rnd.random()
            spd = rnd.choice((rnd.uniform(0, 40), rnd.uniform(0, 3), 1e-20, rnd.uniform(-10, 0)))
            if w < 0.08:
                vel = [0.0, 0.0, 0.0]
            elif w < 0.6:
                vel = [x * spd for x in fwd]
            else:
                vd = yaw + rnd.uniform(-1.6, 1.6)
                vel = [math.sin(vd) * spd, rnd.uniform(-2, 2), -math.cos(vd) * spd]
            lock = 1 if rnd.random() < 0.12 else 0
            frames.append([reset if k == 0 else 0, int(has_b), int(has_rail), t2, lod, nw, lock] +
                          [f32(x) for x in [park, hubz] + focus + pos + R[0] + R[1] + R[2] + [phase, ds, ret6] + tan + [dt] + vel + hy + ang])
    nl = chr(10)
    NL = chr(92) + 'n'
    NF = len(frames[0])
    rows = nl.join('{%s},' % ','.join('0x%08x' % (x if i < NI else fb(x)) for i, x in enumerate(f)) for f in frames)
    s6, s10, sv = 0x82FF0100, 0x82FF0200, 0x82FF0300
    # c[]: 7 park, 8 hub z, 9/10 focus, 11..13 place, 14..22 rows X Y Z, 23 phase, 24 curve speed, 25 slot 6's return,
    # 26..28 slot 10's, 29 dt, 30..32 velocity, 33..38 hub y, 39..44 angles
    disp = (''
            'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'auto flt=[](uint32_t u){float f;memcpy(&f,&u,4);return double(f);};'
            'if(a==0x%x){ctx.f1.f64=flt(cur[25]);}'
            'else if(a==0x%x){store(ctx.r3.u32,cur[26]);store(ctx.r3.u32+4,cur[27]);store(ctx.r3.u32+8,cur[28]);store(ctx.r3.u32+12,0);}'
            'else if(a==0x%x){float f=float(ctx.f25.f64);memcpy(&g25,&f,4);ctx.r3.u32=0x3400;}else throw std::runtime_error("unexpected indirect call");}' % (s6, s10, sv))
    setup = ('memset(objects,0,0x1000);memset(objects+0x1100,0,0x2000-0x1100);memset(objects+0x2000,0,0x1000);'
             'for(int i=0x2000;i<0x2100;i++)if(i<0x2058||i>=0x2084)*pointer(i)=0;'
             'if(c[0]){for(int i=0x2058;i<0x2084;i++)*pointer(i)=0;}'
             'store(0x1000,0x2340);store(0x2340+4,0x%x);' % sv +
             'for(int k=0;k<9;k++)store(0x1000+128+16*(k/3)+4*(k%%3),c[14+k]);'
             'store(0x1000+176,c[11]);store(0x1000+180,c[12]);store(0x1000+184,c[13]);'
             'store(0x1000+208,c[30]);store(0x1000+212,c[31]);store(0x1000+216,c[32]);'
             'store(0x1000+448,0x2200);*pointer(0x2200+112)=c[3];store(0x2200+120,c[5]);store(0x1000+452,c[1]?0x1E40:0);store(0x1000+3584,c[7]);'
             'store(0x1E40+188,c[2]?0x1F40:0);store(0x1E40+208,c[23]);store(0x1E40+212,c[24]);'
             'store(0x1F40,0x1F60);store(0x1F60+24,0x%x);store(0x1F60+40,0x%x);' % (s6, s10) +
             'store(0x2000+4,0x2380);store(0x2000+28,0x2100);store(0x2000+32,c[4]);store(0x2100+16,0x1000);store(0x2100+20,0x2120);'
             'for(int k=0;k<6;k++){store(0x2120+176+4*k,0x3000+0x80*k);store(0x3000+0x80*k+100,c[33+k]);store(0x3000+0x80*k+104,c[8]);store(0x2000+64+4*k,c[39+k]);store(0x2000+36+4*k,0x3800+0x40*k);}'
             'store(0x2280,0x22A0);store(0x2300,c[9]);store(0x2308,c[10]);')
    body = ('static const uint32_t C[][%d]={' % NF + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'for(int k=0;k<3;k++)put(0x82860950+4*k,0xFFFFFFFF);put(0x8286095C,0);for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);' + nl +
            'for(auto&c:C){cur=c;' + setup + 'Context ctx{};ctx.r3.u32=0x2000;ctx.r4.u32=0x1000;ctx.r6.u32=0x2300;'
            'ctx.r13.u32=0x2280;ctx.r1.u32=0x2F00;{float d;memcpy(&d,&c[29],4);ctx.f1.f64=d;}'
            'g25=0xFFFFFFFF;sub_%s(ctx,nullptr);' % upd +
            'printf("%08x",g25);'
            'for(int k=0;k<6;k++)printf(" %08x",load(0x2000+64+4*k));' +
            ('printf("%s");}' % NL) + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    # (the instance matrix getter F-33765 -> the car's +128; the lock F-70315 -> the case's byte; the tilt, compression and
    # drawing callees: empty)
    calls = [addr(a.tool, f) for f in ('F-33765', 'F-70315', 'F-14882', 'F-74033', 'F-97374', 'F-25963', 'F-52786', 'F-83846',
                                       'F-16906', 'F-14577')]
    stubs = {'sub_' + n: '' for n in calls}
    stubs['sub_' + calls[0]] = 'ctx.r3.u32=0x1080;'
    stubs['sub_' + calls[1]] = 'ctx.r3.u64=cur[6];'
    keep += [addr(a.tool, f) for f in ('F-63589', 'F-72580')]
    exe = build(idx, ['sub_' + upd, 'sub_' + ring] + ['sub_' + k for k in keep], main_src, a.out_dir, 'wheel_roll',
                stubs=stubs, vector=True, scratch_end=0x4000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(frames), (len(out), len(frames))
    lines = ['%s -> %s' % (' '.join(('%d' % x) if i < NI else '%08x' % fb(x) for i, x in enumerate(f)), o) for f, o in zip(frames, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'wheel_roll_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'frames,', sum(f[5] for f in frames), 'wheels')


if __name__ == '__main__':
    main()
